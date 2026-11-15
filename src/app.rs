use crate::crash;
use crate::game::{self, Game, Plugin, PluginKind};
use crate::gpu::{self, GpuInfo};
use crate::ipc;
use crate::mods::{self, DeployReport, Library, Mod, NexusRef, Paths};
use crate::needs::{self, Needs};
use crate::nexus::{self, FileInfo, NxmLink, Requirements, SearchMod, SearchPage};
use crate::theme::{self, Palette, icon};
use crate::widgets::{self as w, PillStyle};
use egui::{Align, Align2, Color32, Frame, Layout, Margin, RichText, Ui, vec2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

const SKSE_URL: &str = "https://skse.silverlock.org/";
const NEXUS_URL: &str = nexus::SITE_URL;
const MAX_FULL_PLUGINS: usize = 254;
const MAX_LIGHT_PLUGINS: usize = 4096;
const SEARCH_PAGE: u64 = 20;

#[derive(Default, Serialize, Deserialize)]
struct Settings {
    game_path: Option<PathBuf>,
    /// 0 = system, 1 = light, 2 = dark
    theme: usize,
    /// Personal Nexus Mods API key, kept only on this PC.
    #[serde(default)]
    nexus_key: Option<String>,
    /// Timestamp of the newest crash report the user has already looked at.
    #[serde(default)]
    crash_seen: u64,
}

impl Settings {
    fn load(paths: &Paths) -> Self {
        std::fs::read_to_string(paths.settings_file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save(&self, paths: &Paths) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(paths.settings_file(), s);
        }
    }
    fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.theme {
            1 => egui::ThemePreference::Light,
            2 => egui::ThemePreference::Dark,
            _ => egui::ThemePreference::System,
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Library,
    Browse,
    Plugins,
    Settings,
}

enum Msg {
    Installed(Result<Mod, String>),
    Deployed(Result<(DeployReport, Vec<Plugin>), String>),
    Restored(Result<(), String>),
    /// Arguments handed over by a second copy of the app (nxm links, opened files).
    External(Vec<String>),
    NexusValidated(Result<nexus::User, String>),
    SearchDone { generation: u64, offset: u64, result: Result<SearchPage, String> },
    Thumb(u64, Option<egui::ColorImage>),
    FilesFetched { mod_id: u64, name: String, version: String, replaces: Option<String>, force_pick: bool, result: Result<Vec<FileInfo>, String> },
    DownloadNamed(u64, String),
    Downloaded { id: u64, result: Result<Pending, String> },
    /// Latest version on Nexus for every mod installed from there: (mod_id, version).
    UpdatesChecked(Vec<(u64, String)>),
    /// Requirements for the mods asked about; a failure frees them to be asked again.
    Requirements { asked: Vec<u64>, result: Result<HashMap<u64, Requirements>, String> },
}

/// A row being carried to a new place in one of the lists.
#[derive(Clone, Copy)]
struct Drag {
    page: Page,
    /// Index in the full (unfiltered) list.
    from: usize,
    /// True once the pointer has moved enough for this to be a drag rather than a click.
    decided: bool,
}

/// Where a dragged row would land among the rows on screen: the slot before the first
/// row whose middle is below the pointer, or after the last one.
fn drop_slot(rows: &[(usize, egui::Rect)], y: f32) -> usize {
    rows.iter().position(|(_, r)| y < r.center().y).unwrap_or(rows.len())
}

/// Moves `from` so it sits just before what is currently at `before` (`len` = append).
fn move_item<T>(list: &mut Vec<T>, from: usize, before: usize) {
    let to = if before > from { before - 1 } else { before };
    if from != to {
        let item = list.remove(from);
        list.insert(to, item);
    }
}

/// A file waiting to be installed, with anything we already know about it.
#[derive(Clone)]
pub struct Pending {
    path: PathBuf,
    /// Name to use instead of one guessed from the file name.
    name: Option<String>,
    nexus: Option<NexusRef>,
    /// Library entry this install replaces (an update keeps its position and switch).
    replaces: Option<String>,
    /// Delete the archive afterwards (downloads live in our own folder).
    cleanup: bool,
}

impl Pending {
    fn from_path(path: PathBuf) -> Self {
        Self { path, name: None, nexus: None, replaces: None, cleanup: false }
    }
    fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            self.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
        })
    }
}

/// Which Library entry a fresh install of a Nexus file should take the place of, so a mod
/// doesn't end up in the Library twice. `existing` is (Library id, file id) for everything
/// already installed from the mod; `files` is the mod's file list from Nexus.
///
/// The very same file reinstalls over its entry. A mod's one main file replaces the main
/// file installed before it (an update) — or whatever was installed if Nexus no longer
/// lists it. Optional files and the parts of a multi-part mod stay separate.
fn replacement_for(existing: &[(String, u64)], files: &[FileInfo], file_id: u64, multi_part: bool) -> Option<String> {
    if let Some((id, _)) = existing.iter().find(|(_, f)| *f == file_id) {
        return Some(id.clone());
    }
    let new = files.iter().find(|f| f.file_id == file_id)?;
    if !new.is_main() || multi_part {
        return None;
    }
    let was_main = |fid: u64| files.iter().find(|f| f.file_id == fid).is_none_or(|f| f.is_main() || f.category() == "OLD_VERSION");
    existing.iter().find(|(_, f)| was_main(*f)).map(|(id, _)| id.clone())
}

/// Library name for a Nexus file. A plain single-file install keeps the mod's name; a part
/// of a multi-file mod (or an optional file) says which part it is so they don't all look alike.
fn install_title(name: &str, file: Option<&FileInfo>, multi_part: bool) -> String {
    match file {
        Some(f) if (multi_part || !f.is_main()) && !f.name.trim().is_empty() => {
            let part = f.name.trim().trim_start_matches(['-', '_', ' ']).trim();
            // "Skyrim 202X 10.5.2 - Architecture PART 1" already says it's Skyrim 202X.
            let stem: String = name.split_whitespace().take(2).collect::<Vec<_>>().join(" ").to_ascii_lowercase();
            if !stem.is_empty() && part.to_ascii_lowercase().starts_with(&stem) {
                part.to_string()
            } else {
                format!("{name} · {part}")
            }
        }
        _ => name.to_string(),
    }
}

struct Download {
    id: u64,
    mod_id: u64,
    file_id: u64,
    name: String,
    progress: Arc<nexus::Progress>,
}

/// "Which file do you want?" — shown when a mod has more than one main file.
struct FilePick {
    mod_id: u64,
    name: String,
    version: String,
    replaces: Option<String>,
    files: Vec<FileInfo>,
}

#[derive(Default)]
struct Browse {
    /// Bumped for every new query so late replies to old searches are dropped.
    generation: u64,
    /// What the user has typed so far, and when they last typed (debounce).
    typed: String,
    last_typed: f64,
    /// The query the current results belong to.
    query: Option<String>,
    results: Vec<SearchMod>,
    total: u64,
    loading: bool,
    error: Option<String>,
}

/// What a mod needs that isn't there: other mods' plugins, the script extender, its offsets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Missing {
    /// Master plugins no mod in the Library provides ("SkyUI_SE.esp").
    masters: Vec<String>,
    /// Library mods that provide a needed plugin but are switched off, by name.
    off: Vec<String>,
    skse: bool,
    address_library: bool,
    /// SKSE plugins in the mod that were built for another version of the game.
    outdated: Vec<needs::Outdated>,
}

impl Missing {
    fn is_empty(&self) -> bool {
        !self.lacks_anything() && self.outdated.is_empty()
    }
    /// Something it depends on isn't there (as opposed to the mod itself being out of date).
    fn lacks_anything(&self) -> bool {
        !self.masters.is_empty() || !self.off.is_empty() || self.skse || self.address_library
    }
    /// "Made for Skyrim 1.6.1170 — won't load on 1.7.104", when a plugin is out of date.
    fn outdated_text(&self, game: &str) -> Option<String> {
        let o = self.outdated.iter().find(|o| o.certain).or(self.outdated.first())?;
        let outcome = if o.certain { "won't load on" } else { "may not work on" };
        Some(format!("Made for Skyrim {} — {outcome} {game}", o.made_for))
    }
    /// "SKSE, Address Library and SkyUI (turned off)"
    fn list(&self) -> String {
        let mut items: Vec<String> = Vec::new();
        if self.skse {
            items.push(needs::SKSE_NAME.into());
        }
        if self.address_library {
            items.push(needs::ADDRESS_LIBRARY_NAME.into());
        }
        items.extend(self.masters.iter().map(|m| needs::display_name(m)));
        items.extend(self.off.iter().map(|m| format!("{m} (turned off)")));
        join_names(&items)
    }
}

/// Where a plugin another mod depends on comes from.
enum Provider<'a> {
    /// The game itself, so it's always there.
    Game,
    /// A mod in the Library — which may be switched off.
    Mod(&'a Mod),
    /// Nothing we know of.
    Nobody,
}

fn push_unique(items: &mut Vec<String>, s: String) {
    if !items.iter().any(|x| x.eq_ignore_ascii_case(&s)) {
        items.push(s);
    }
}

/// The longer story behind an out-of-date plugin, for a tooltip.
fn outdated_hint(lacking: &Missing, game: &str) -> String {
    let dlls: Vec<String> = lacking.outdated.iter().map(|o| o.dll.clone()).collect();
    let certain = lacking.outdated.iter().any(|o| o.certain);
    let outcome = if certain {
        "SKSE checks every plugin against the game before loading it, and will leave this one out."
    } else {
        "The game's offsets format changed in 1.7.99, and this build doesn't say it's ready for it — plugins like this usually stop with an \"address library\" error."
    };
    format!("{} {outcome} Look for a version made for Skyrim {game} on Nexus Mods.", join_names(&dlls))
}

/// "A", "A and B", "A, B and C".
fn join_names(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The newest crash report, with each file it names traced back to a Library mod.
struct Crash {
    report: crash::Report,
    /// (file the log named, the mod it belongs to if we installed it)
    suspects: Vec<(String, Option<String>)>,
}

#[derive(Clone)]
enum Busy {
    Installing(String),
    Applying,
    Restoring,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToastKind {
    Success,
    Error,
}

struct Toast {
    text: String,
    kind: ToastKind,
    until: f64,
}

pub struct App {
    paths: Paths,
    settings: Settings,
    game: Option<Game>,
    library: Library,
    plugins: Vec<Plugin>,
    page: Page,
    search: String,
    busy: Option<Busy>,
    needs_apply: bool,
    apply_failed: bool,
    pending_installs: VecDeque<Pending>,
    /// The install currently running, so its details can be applied when it finishes.
    installing: Option<Pending>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    toast: Option<Toast>,
    confirm_remove: Option<String>,
    confirm_restore: bool,
    last_report: Option<DeployReport>,
    show_core: bool,
    /// A Library or Load Order row being dragged to a new spot.
    drag: Option<Drag>,
    gpu: Option<GpuInfo>,
    /// DLSS runtimes found in the game folder; refreshed whenever the game folder changes.
    runtimes: Vec<gpu::Runtime>,

    // Nexus Mods
    nexus: Option<Arc<nexus::Client>>,
    nexus_user: Option<nexus::User>,
    connecting: bool,
    key_input: String,
    /// A key being checked; stored in settings once Nexus accepts it.
    pending_key: Option<String>,
    /// nxm links that arrived before the account was connected.
    pending_links: Vec<NxmLink>,
    downloads: Vec<Download>,
    next_download: u64,
    /// Mods whose file list is being fetched.
    fetching_files: HashSet<u64>,
    file_pick: Option<FilePick>,
    browse: Browse,
    /// Mod previews by Nexus mod id, shared by Browse and the Library.
    /// `None` while loading or when the image couldn't be fetched.
    thumbs: HashMap<u64, Option<egui::TextureHandle>>,
    /// Latest version on Nexus, by mod id, for mods installed from there.
    latest_versions: HashMap<u64, String>,
    protocol_ok: bool,
    /// Newest crash report the user hasn't dismissed yet.
    crash: Option<Crash>,
    show_crash: bool,
    /// When the crash folder was last looked at (egui time).
    crash_checked: f64,
    /// What each Library mod depends on, by mod id, read from its files.
    needs: HashMap<String, Needs>,
    /// SkyrimSE.exe's version ("1.7.104.0"), which decides the Address Library file needed.
    game_version: Option<String>,
    /// What each mod's Nexus page lists under Requirements, by mod id. `None` while loading.
    requirements: HashMap<u64, Option<Requirements>>,
    /// The mod whose requirements are open in a sheet: (mod id, name).
    req_sheet: Option<(u64, String)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Vec<String>, listener: Option<ipc::Listener>) -> anyhow::Result<Self> {
        let paths = Paths::new()?;
        let settings = Settings::load(&paths);
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx);
        settings.apply_theme(&cc.egui_ctx);

        let game = settings
            .game_path
            .as_deref()
            .and_then(Game::from_user_path)
            .or_else(Game::detect);
        let plugins = game.as_ref().map(game::scan_plugins).unwrap_or_default();
        let library = Library::load(&paths);
        let (tx, rx) = channel();

        if let Some(listener) = listener {
            let tx = tx.clone();
            let ctx = cc.egui_ctx.clone();
            listener.run(move |args| {
                let _ = tx.send(Msg::External(args));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            });
        }

        let mut app = Self {
            paths,
            settings,
            library,
            plugins,
            page: Page::Library,
            search: String::new(),
            busy: None,
            needs_apply: false,
            apply_failed: false,
            pending_installs: VecDeque::new(),
            installing: None,
            tx,
            rx,
            toast: None,
            confirm_remove: None,
            confirm_restore: false,
            last_report: None,
            show_core: false,
            drag: None,
            gpu: gpu::detect(),
            runtimes: game.as_ref().map(gpu::installed_runtimes).unwrap_or_default(),
            game,
            nexus: None,
            nexus_user: None,
            connecting: false,
            key_input: String::new(),
            pending_key: None,
            pending_links: Vec::new(),
            downloads: Vec::new(),
            next_download: 1,
            fetching_files: HashSet::new(),
            file_pick: None,
            browse: Browse::default(),
            thumbs: HashMap::new(),
            latest_versions: HashMap::new(),
            protocol_ok: nexus::current_exe().is_some_and(|e| nexus::protocol_registered(&e)),
            crash: None,
            show_crash: false,
            crash_checked: f64::NEG_INFINITY,
            needs: HashMap::new(),
            game_version: None,
            requirements: HashMap::new(),
            req_sheet: None,
        };
        // Cached previews show straight away, even before (or without) a Nexus account.
        app.request_library_thumbs(&cc.egui_ctx);
        app.refresh_crash();
        app.refresh_game_version();
        app.refresh_needs();
        if let Some(key) = app.settings.nexus_key.clone() {
            app.connect(&cc.egui_ctx, &key, false);
        }
        app.handle_external(&cc.egui_ctx, args);
        Ok(app)
    }

    fn refresh_runtimes(&mut self) {
        self.runtimes = self.game.as_ref().map(gpu::installed_runtimes).unwrap_or_default();
    }

    fn refresh_game_version(&mut self) {
        self.game_version = self.game.as_ref().and_then(|g| gpu::file_version(&g.root.join("SkyrimSE.exe")));
    }

    /// Re-reads what every Library mod depends on. Cheap enough to run after each change.
    fn refresh_needs(&mut self) {
        self.needs = self
            .library
            .mods
            .iter()
            .map(|m| (m.id.clone(), needs::scan(&self.paths.mod_dir(&m.id))))
            .collect();
    }

    /// The script extender is in place, or an enabled mod will put it there.
    fn skse_available(&self) -> bool {
        self.game.as_ref().is_some_and(|g| g.skse().is_some())
            || self.library.mods.iter().any(|m| {
                m.enabled && m.root_files.iter().any(|f| f.eq_ignore_ascii_case("skse64_loader.exe"))
            })
    }

    /// Address Library's offsets file for this game version is in place, or on its way there.
    fn address_library_available(&self) -> bool {
        let Some(v) = &self.game_version else { return true };
        let file = needs::versionlib_for(v);
        self.game.as_ref().is_some_and(|g| g.data_dir().join("SKSE").join("Plugins").join(&file).is_file())
            || self.library.mods.iter().any(|m| {
                m.enabled && self.needs.get(&m.id).is_some_and(|n| n.versionlibs.contains(&file))
            })
    }

    /// Who provides a plugin: the game, a Library mod (an enabled one if there's a choice), or nobody.
    fn provider_of(&self, plugin: &str) -> Provider<'_> {
        if self.plugins.iter().any(|p| p.core && p.name.eq_ignore_ascii_case(plugin)) {
            return Provider::Game;
        }
        self.library
            .mods
            .iter()
            .filter(|o| o.plugins.iter().any(|p| p.eq_ignore_ascii_case(plugin)))
            .max_by_key(|o| o.enabled)
            .map_or(Provider::Nobody, Provider::Mod)
    }

    /// "SKSE, Address Library and SkyUI" — everything a mod depends on, naming the mods that
    /// bring its master plugins where we know them. Plugins the game ships are left out.
    fn needs_text(&self, n: &Needs) -> String {
        let mut items: Vec<String> = Vec::new();
        if n.skse {
            items.push(needs::SKSE_NAME.into());
        }
        if n.address_library {
            items.push(needs::ADDRESS_LIBRARY_NAME.into());
        }
        for master in &n.masters {
            match self.provider_of(master) {
                Provider::Game => {}
                Provider::Mod(o) => push_unique(&mut items, o.name.clone()),
                Provider::Nobody => push_unique(&mut items, needs::display_name(master)),
            }
        }
        join_names(&items)
    }

    /// The game's version the way mod pages write it: "1.7.104".
    fn game_label(&self) -> String {
        let v = self.game_version.as_deref().unwrap_or_default();
        needs::parse_version(v).map(needs::format_version).unwrap_or_else(|| v.to_string())
    }

    /// The mod's SKSE plugins that were built for another version of the game.
    fn outdated_for(&self, m: &Mod) -> Vec<needs::Outdated> {
        let Some(game) = self.game_version.as_deref().and_then(needs::parse_version) else { return Vec::new() };
        let Some(n) = self.needs.get(&m.id) else { return Vec::new() };
        n.plugins.iter().filter_map(|p| p.fit(game)).collect()
    }

    /// Everything this mod needs that no enabled mod (or the game) provides.
    fn missing_for(&self, m: &Mod) -> Missing {
        let Some(n) = self.needs.get(&m.id) else { return Missing::default() };
        let mut missing = Missing {
            skse: n.skse && !self.skse_available(),
            address_library: n.address_library && !self.address_library_available(),
            outdated: self.outdated_for(m),
            ..Missing::default()
        };
        for master in &n.masters {
            match self.provider_of(master) {
                Provider::Game => {}
                Provider::Mod(o) if o.enabled => {}
                Provider::Mod(o) => push_unique(&mut missing.off, o.name.clone()),
                Provider::Nobody => push_unique(&mut missing.masters, master.clone()),
            }
        }
        missing
    }

    // -----------------------------------------------------------------------
    // Background work
    // -----------------------------------------------------------------------

    fn toast(&mut self, ctx: &egui::Context, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        let secs = if kind == ToastKind::Error { 6.0 } else { 3.0 };
        if kind == ToastKind::Error {
            self.paths.log(&format!("error: {text}"));
        }
        self.toast = Some(Toast {
            text,
            kind,
            until: ctx.input(|i| i.time) + secs,
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Installed(Ok(mut m)) => {
                    self.busy = None;
                    let meta = self.installing.take();
                    let mut updated = false;
                    if let Some(meta) = &meta {
                        if let Some(n) = &meta.name {
                            m.name = n.clone();
                        }
                        m.nexus = meta.nexus.clone();
                        if meta.cleanup {
                            let _ = std::fs::remove_file(&meta.path);
                        }
                    }
                    let old = meta
                        .as_ref()
                        .and_then(|p| p.replaces.as_ref())
                        .and_then(|id| self.library.mods.iter().position(|x| &x.id == id));
                    let name = m.name.clone();
                    let mut same_file = false;
                    match old {
                        Some(i) => {
                            let prev = self.library.mods.remove(i);
                            m.enabled = prev.enabled;
                            same_file = prev.nexus.as_ref().map(|n| n.file_id) == m.nexus.as_ref().map(|n| n.file_id);
                            self.library.mods.insert(i, m);
                            mods::remove_mod_files(&self.paths, &prev.id);
                            updated = true;
                        }
                        None => self.library.mods.push(m),
                    }
                    let _ = self.library.save(&self.paths);
                    self.needs_apply = true;
                    self.refresh_needs();
                    self.request_library_thumbs(ctx);
                    let text = match (updated, same_file) {
                        (true, true) => format!("Reinstalled {name}"),
                        (true, false) => format!("Updated {name}"),
                        _ => format!("Installed {name}"),
                    };
                    self.toast(ctx, ToastKind::Success, text);
                }
                Msg::Installed(Err(e)) => {
                    self.busy = None;
                    // Keep the downloaded archive so a failed install can be retried by hand.
                    let meta = self.installing.take();
                    let e = match meta.filter(|m| m.cleanup) {
                        Some(m) => format!("{e} — the download was kept at {}", m.path.display()),
                        None => e,
                    };
                    self.toast(ctx, ToastKind::Error, e)
                }
                Msg::External(args) => self.handle_external(ctx, args),
                Msg::NexusValidated(result) => {
                    self.connecting = false;
                    match result {
                        Ok(user) => {
                            self.toast(ctx, ToastKind::Success, format!("Connected to Nexus Mods as {}", user.name));
                            self.nexus_user = Some(user);
                            self.key_input.clear();
                            if let Some(key) = self.pending_key.take() {
                                self.settings.nexus_key = Some(key);
                                self.settings.save(&self.paths);
                            }
                            if !self.protocol_ok
                                && let Some(exe) = nexus::current_exe()
                            {
                                self.protocol_ok = nexus::register_protocol(&exe).is_ok();
                            }
                            for link in std::mem::take(&mut self.pending_links) {
                                self.handle_link(ctx, link);
                            }
                            self.check_updates(ctx);
                            self.request_library_thumbs(ctx);
                            if self.page == Page::Browse {
                                self.run_search(ctx, 0);
                            }
                        }
                        Err(e) => {
                            self.nexus = None;
                            self.pending_key = None;
                            // A revoked key stays broken; a network hiccup doesn't.
                            if e.contains("isn't valid") && self.settings.nexus_key.take().is_some() {
                                self.settings.save(&self.paths);
                            }
                            self.toast(ctx, ToastKind::Error, e);
                        }
                    }
                }
                Msg::SearchDone { generation, offset, result } => {
                    if generation != self.browse.generation {
                        continue;
                    }
                    self.browse.loading = false;
                    match result {
                        Ok(page) => {
                            if offset == 0 {
                                self.browse.results.clear();
                            }
                            self.browse.total = page.total;
                            self.browse.error = None;
                            let wanted = page.mods.iter().map(|m| (m.mod_id, m.thumbnail_url.clone())).collect();
                            self.request_thumbs(ctx, wanted);
                            self.request_requirements(ctx, page.mods.iter().map(|m| m.mod_id).collect());
                            self.browse.results.extend(page.mods);
                        }
                        Err(e) => self.browse.error = Some(e),
                    }
                }
                Msg::Thumb(mod_id, image) => {
                    let tex = image.map(|img| ctx.load_texture(format!("thumb-{mod_id}"), img, egui::TextureOptions::LINEAR));
                    self.thumbs.insert(mod_id, tex);
                }
                Msg::Requirements { asked, result } => match result {
                    Ok(found) => {
                        // A mod Nexus didn't answer for has no page to read; treat it as needing nothing.
                        for id in asked {
                            self.requirements.insert(id, Some(found.get(&id).cloned().unwrap_or_default()));
                        }
                    }
                    Err(e) => {
                        for id in asked {
                            self.requirements.remove(&id);
                        }
                        if self.req_sheet.is_some() {
                            self.toast(ctx, ToastKind::Error, e);
                        }
                    }
                },
                Msg::FilesFetched { mod_id, name, version, replaces, force_pick, result } => {
                    self.fetching_files.remove(&mod_id);
                    match result {
                        Ok(files) => {
                            let current: Vec<FileInfo> = files.into_iter().filter(FileInfo::is_current).collect();
                            let mains: Vec<&FileInfo> = current.iter().filter(|f| f.is_main()).collect();
                            let auto = if force_pick {
                                None
                            } else if mains.len() == 1 {
                                Some(mains[0].clone())
                            } else if mains.is_empty() && current.len() == 1 {
                                Some(current[0].clone())
                            } else {
                                None
                            };
                            match auto {
                                Some(f) => self.start_download(ctx, mod_id, &name, &version, Some(f), None, replaces, false),
                                None if current.is_empty() => {
                                    self.toast(ctx, ToastKind::Error, format!("{name} has no files to download"))
                                }
                                None => self.file_pick = Some(FilePick { mod_id, name, version, replaces, files: current }),
                            }
                        }
                        Err(e) => self.toast(ctx, ToastKind::Error, e),
                    }
                }
                Msg::DownloadNamed(id, name) => {
                    if let Some(d) = self.downloads.iter_mut().find(|d| d.id == id) {
                        d.name = name;
                    }
                }
                Msg::Downloaded { id, result } => {
                    self.downloads.retain(|d| d.id != id);
                    match result {
                        Ok(pending) => self.pending_installs.push_back(pending),
                        Err(e) => self.toast(ctx, ToastKind::Error, e),
                    }
                }
                Msg::UpdatesChecked(latest) => {
                    self.latest_versions = latest.into_iter().collect();
                    let n = self.library.mods.iter().filter(|m| self.update_for(m).is_some()).count();
                    if n > 0 {
                        self.toast(ctx, ToastKind::Success, format!("{n} {} an update", if n == 1 { "mod has" } else { "mods have" }));
                    }
                }
                Msg::Deployed(Ok((report, plugins))) => {
                    self.busy = None;
                    self.plugins = plugins;
                    self.apply_failed = false;
                    self.last_report = Some(report);
                    self.refresh_runtimes();
                }
                Msg::Deployed(Err(e)) => {
                    self.busy = None;
                    self.apply_failed = true;
                    self.needs_apply = false;
                    self.toast(ctx, ToastKind::Error, format!("Couldn't apply mods: {e}"));
                }
                Msg::Restored(Ok(())) => {
                    self.busy = None;
                    if let Some(g) = &self.game {
                        self.plugins = game::scan_plugins(g);
                        let _ = game::write_plugins_txt(&self.plugins);
                    }
                    self.refresh_runtimes();
                    self.toast(ctx, ToastKind::Success, "Game folder restored to vanilla");
                }
                Msg::Restored(Err(e)) => {
                    self.busy = None;
                    self.toast(ctx, ToastKind::Error, e)
                }
            }
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        // A crash report can appear at any time while the game runs; glance at the folder
        // now and then (one directory read) so it shows up without a restart.
        let now = ctx.input(|i| i.time);
        if now - self.crash_checked > 5.0 {
            self.crash_checked = now;
            self.refresh_crash();
            ctx.request_repaint_after(std::time::Duration::from_secs(5));
        }
        if self.busy.is_some() {
            return;
        }
        if let Some(pending) = self.pending_installs.pop_front() {
            let path = pending.path.clone();
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            self.busy = Some(Busy::Installing(pending.label()));
            self.installing = Some(pending);
            let tx = self.tx.clone();
            let paths = self.paths.clone();
            let ctx = ctx.clone();
            let loose = mods::is_loose_file(&path);
            let into = self.game.as_ref().and_then(|g| gpu::runtime_home(g, &name));
            std::thread::spawn(move || {
                let r = if loose {
                    mods::install_file(&paths, &path, into.as_deref())
                } else {
                    mods::install(&paths, &path)
                }
                .map_err(|e| e.to_string());
                let _ = tx.send(Msg::Installed(r));
                ctx.request_repaint();
            });
            return;
        }
        if self.needs_apply
            && let Some(game) = self.game.clone()
        {
            self.needs_apply = false;
            self.busy = Some(Busy::Applying);
            let tx = self.tx.clone();
            let paths = self.paths.clone();
            let library = self.library.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let r = mods::deploy(&paths, &game, &library).map_err(|e| e.to_string());
                let _ = tx.send(Msg::Deployed(r));
                ctx.request_repaint();
            });
        }
    }

    fn queue_install(&mut self, path: PathBuf) {
        self.pending_installs.push_back(Pending::from_path(path));
    }

    // -----------------------------------------------------------------------
    // Nexus Mods
    // -----------------------------------------------------------------------

    fn handle_external(&mut self, ctx: &egui::Context, args: Vec<String>) {
        for a in args {
            if a.starts_with("nxm://") {
                match NxmLink::parse(&a) {
                    Some(link) => self.handle_link(ctx, link),
                    None => self.toast(ctx, ToastKind::Error, "That Nexus link isn't a mod file"),
                }
            } else {
                let p = PathBuf::from(&a);
                if p.exists() {
                    self.queue_install(p);
                }
            }
        }
    }

    fn handle_link(&mut self, ctx: &egui::Context, link: NxmLink) {
        if link.game != nexus::GAME {
            self.toast(ctx, ToastKind::Error, "That download is for a different game");
            return;
        }
        if self.nexus.is_none() || (self.nexus_user.is_none() && !self.connecting) {
            self.pending_links.push(link);
            self.page = Page::Settings;
            self.toast(ctx, ToastKind::Error, "Connect your Nexus Mods account to download this mod");
            return;
        }
        if self.connecting {
            self.pending_links.push(link);
            return;
        }
        if self.downloads.iter().any(|d| d.mod_id == link.mod_id) {
            return;
        }
        let (mod_id, file_id) = (link.mod_id, link.file_id);
        // Clicking the website's download button for a file that's already here means
        // "install it again", not "give me a second copy".
        let already = self
            .library
            .mods
            .iter()
            .find(|m| m.nexus.as_ref().is_some_and(|n| n.mod_id == mod_id && n.file_id == file_id))
            .map(|m| m.name.clone());
        if let Some(name) = already {
            self.toast(ctx, ToastKind::Success, format!("{name} is already installed — installing it again"));
        }
        self.start_download(ctx, mod_id, "", "", None, Some((file_id, link.key, link.expires)), None, false);
    }

    fn connect(&mut self, ctx: &egui::Context, key: &str, save: bool) {
        let key = key.trim().to_string();
        if key.is_empty() {
            return;
        }
        let client = match nexus::Client::new(&key) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                self.toast(ctx, ToastKind::Error, e.to_string());
                return;
            }
        };
        self.pending_key = save.then_some(key);
        self.nexus = Some(client.clone());
        self.nexus_user = None;
        self.connecting = true;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = client.validate().map_err(|e| e.to_string());
            let _ = tx.send(Msg::NexusValidated(r));
            ctx.request_repaint();
        });
    }

    fn disconnect(&mut self) {
        self.nexus = None;
        self.nexus_user = None;
        self.connecting = false;
        self.settings.nexus_key = None;
        self.settings.save(&self.paths);
        self.latest_versions.clear();
        self.browse = Browse::default();
    }

    fn is_premium(&self) -> bool {
        self.nexus_user.as_ref().is_some_and(|u| u.is_premium)
    }

    /// Fetches a mod's file list and either downloads the obvious main file or asks which one.
    fn install_from_nexus(&mut self, ctx: &egui::Context, mod_id: u64, name: &str, version: &str, replaces: Option<String>, force_pick: bool) {
        let Some(client) = self.nexus.clone() else { return };
        if !self.fetching_files.insert(mod_id) {
            return;
        }
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let (name, version) = (name.to_string(), version.to_string());
        std::thread::spawn(move || {
            let result = client.files(mod_id).map_err(|e| e.to_string());
            let _ = tx.send(Msg::FilesFetched { mod_id, name, version, replaces, force_pick, result });
            ctx.request_repaint();
        });
    }

    /// Downloads one file from Nexus in the background, then queues it for install.
    /// `file` is the chosen file when known; `link` carries the id (and free-account key)
    /// from an nxm link instead.
    fn start_download(
        &mut self,
        ctx: &egui::Context,
        mod_id: u64,
        name: &str,
        version: &str,
        file: Option<FileInfo>,
        link: Option<(u64, Option<String>, Option<u64>)>,
        replaces: Option<String>,
        picked: bool,
    ) {
        let Some(client) = self.nexus.clone() else { return };
        let id = self.next_download;
        self.next_download += 1;
        let progress = Arc::new(nexus::Progress::default());
        let label = if name.is_empty() {
            "Nexus Mods download".to_string()
        } else {
            install_title(name, file.as_ref(), picked)
        };
        let file_id = link.as_ref().map(|l| l.0).or(file.as_ref().map(|f| f.file_id)).unwrap_or(0);
        self.downloads.push(Download { id, mod_id, file_id, name: label, progress: progress.clone() });

        // What's already in the Library from this mod, so it doesn't end up there twice.
        let existing: Vec<(String, u64)> = self
            .library
            .mods
            .iter()
            .filter_map(|m| m.nexus.as_ref().filter(|n| n.mod_id == mod_id).map(|n| (m.id.clone(), n.file_id)))
            .collect();
        let mut replaces = replaces.or_else(|| existing.iter().find(|(_, f)| *f == file_id).map(|(id, _)| id.clone()));

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let dir = self.paths.downloads_dir();
        let (mut name, mut version) = (name.to_string(), version.to_string());
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<Pending> {
                if name.is_empty() || version.is_empty() {
                    let info = client.mod_info(mod_id)?;
                    name = info.name;
                    version = info.version;
                    let _ = tx.send(Msg::DownloadNamed(id, name.clone()));
                    ctx.request_repaint();
                }
                let (file_id, key, expires) = match &link {
                    Some((f, k, e)) => (*f, k.clone(), *e),
                    None => (file.as_ref().map(|f| f.file_id).unwrap_or(0), None, None),
                };
                // From an nxm link we only know the file id; look the file up for its name,
                // and note whether it's one of several main files (parts of a big mod). The
                // list also tells whether this file is a newer take on one already installed.
                let mut multi_part = picked;
                let mut file = file;
                if file.is_none() || (replaces.is_none() && !existing.is_empty()) {
                    if let Ok(fs) = client.files(mod_id) {
                        multi_part |= fs.iter().filter(|f| f.is_main() && f.is_current()).count() > 1;
                        if replaces.is_none() {
                            replaces = replacement_for(&existing, &fs, file_id, multi_part);
                        }
                        if file.is_none() {
                            file = fs.into_iter().find(|f| f.file_id == file_id);
                        }
                    }
                }
                let url = client.download_link(mod_id, file_id, key.as_deref(), expires)?;
                let from_url = url
                    .split('?')
                    .next()
                    .and_then(|u| u.rsplit('/').next())
                    .map(|s| s.replace("%20", " "))
                    .filter(|s| !s.is_empty());
                let file_name = file
                    .as_ref()
                    .map(|f| f.file_name.clone())
                    .filter(|s| !s.is_empty())
                    .or(from_url)
                    .unwrap_or_else(|| format!("{mod_id}-{file_id}.zip"));
                let dest = dir.join(file_name);
                client.download(&url, &dest, &progress)?;
                let title = install_title(&name, file.as_ref(), multi_part);
                Ok(Pending {
                    path: dest,
                    name: Some(title),
                    nexus: Some(NexusRef { mod_id, file_id, version: version.clone() }),
                    replaces,
                    cleanup: true,
                })
            })()
            .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Downloaded { id, result });
            ctx.request_repaint();
        });
    }

    fn run_search(&mut self, ctx: &egui::Context, offset: u64) {
        let Some(client) = self.nexus.clone() else { return };
        let query = self.browse.typed.trim().to_string();
        if offset == 0 {
            self.browse.generation += 1;
            self.browse.query = Some(query.clone());
        }
        self.browse.loading = true;
        self.browse.error = None;
        let generation = self.browse.generation;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = client.search(&query, offset, SEARCH_PAGE).map_err(|e| e.to_string());
            let _ = tx.send(Msg::SearchDone { generation, offset, result });
            ctx.request_repaint();
        });
    }

    /// Previews for every mod in the Library that came from Nexus. Cached ones load
    /// from disk without an account; the rest are fetched once we're connected.
    fn request_library_thumbs(&mut self, ctx: &egui::Context) {
        let online = self.nexus.is_some();
        let wanted: Vec<(u64, Option<String>)> = self
            .library
            .mods
            .iter()
            .filter_map(|m| m.nexus.as_ref().map(|n| n.mod_id))
            .filter(|id| online || self.paths.thumb_file(*id).exists())
            .map(|id| (id, None))
            .collect();
        self.request_thumbs(ctx, wanted);
    }

    /// Loads previews for `(mod id, image url)` pairs not already known. Without a
    /// url the mod's page is asked for one, which needs a connected account.
    fn request_thumbs(&mut self, ctx: &egui::Context, wanted: Vec<(u64, Option<String>)>) {
        let wanted: Vec<(u64, Option<String>)> = wanted.into_iter().filter(|(id, _)| !self.thumbs.contains_key(id)).collect();
        if wanted.is_empty() {
            return;
        }
        for (id, _) in &wanted {
            self.thumbs.insert(*id, None);
        }
        let client = self.nexus.clone();
        let paths = self.paths.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for (id, url) in wanted {
                let image = Self::load_thumb(&paths, client.as_deref(), id, url);
                let _ = tx.send(Msg::Thumb(id, image));
                ctx.request_repaint();
            }
        });
    }

    /// Asks Nexus what these mods need, skipping ones already known or being fetched.
    fn request_requirements(&mut self, ctx: &egui::Context, ids: Vec<u64>) {
        let Some(client) = self.nexus.clone() else { return };
        let asked: Vec<u64> = ids.into_iter().filter(|id| !self.requirements.contains_key(id)).collect();
        if asked.is_empty() {
            return;
        }
        for id in &asked {
            self.requirements.insert(*id, None);
        }
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = client.requirements(&asked).map_err(|e| e.to_string());
            let _ = tx.send(Msg::Requirements { asked, result });
            ctx.request_repaint();
        });
    }

    /// Whether a Nexus requirement is already taken care of: installed from Nexus, or —
    /// for SKSE and Address Library — in place however it got there.
    fn requirement_met(&self, r: &nexus::Requirement) -> Option<bool> {
        match r.mod_id {
            0 => None,
            nexus::SKSE_MOD_ID => Some(self.skse_available()),
            needs::ADDRESS_LIBRARY_MOD_ID => Some(self.address_library_available()),
            id => Some(self.installed_from(id).is_some_and(|m| m.enabled)),
        }
    }

    /// "SKSE, Address Library and SkyUI" from a mod's Nexus page, and whether any of it is
    /// missing. Names are shortened where we have our own.
    fn requirements_text(&self, reqs: &Requirements) -> (String, bool) {
        let mut items: Vec<String> = Vec::new();
        let mut lacking = false;
        for r in &reqs.mods {
            let name = match r.mod_id {
                nexus::SKSE_MOD_ID => needs::SKSE_NAME.to_string(),
                needs::ADDRESS_LIBRARY_MOD_ID => needs::ADDRESS_LIBRARY_NAME.to_string(),
                _ => r.name.clone(),
            };
            push_unique(&mut items, name);
            lacking |= self.requirement_met(r) == Some(false);
        }
        for d in &reqs.dlc {
            push_unique(&mut items, d.clone());
        }
        (join_names(&items), lacking)
    }

    /// The cached preview for a mod, or a freshly fetched one that's then cached
    /// at a small fixed size so the Library keeps its pictures offline.
    fn load_thumb(paths: &Paths, client: Option<&nexus::Client>, id: u64, url: Option<String>) -> Option<egui::ColorImage> {
        let file = paths.thumb_file(id);
        let cached = std::fs::read(&file).ok().and_then(|b| image::load_from_memory(&b).ok());
        let img = match cached {
            Some(img) => img,
            None => {
                let client = client?;
                let url = match url {
                    Some(u) => u,
                    None => client.mod_info(id).ok()?.picture_url?,
                };
                let bytes = client.fetch_bytes(&url).ok()?;
                let img = image::load_from_memory(&bytes).ok()?.thumbnail(240, 135);
                let _ = img.save(&file);
                img
            }
        };
        let img = img.to_rgba8();
        let (w, h) = img.dimensions();
        Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &img.into_raw()))
    }

    /// Picks up a crash report the user hasn't seen yet and works out which mods it points at.
    fn refresh_crash(&mut self) {
        let Some(report) = crash::find_reports().into_iter().next() else { return };
        if report.time <= self.settings.crash_seen || self.crash.as_ref().is_some_and(|c| c.report == report) {
            return;
        }
        let suspects = report.culprits.iter().map(|f| (f.clone(), self.mod_for_file(f))).collect();
        self.crash = Some(Crash { report, suspects });
    }

    /// The Library mod that ships a file with this name (a plugin or an SKSE DLL).
    fn mod_for_file(&self, file: &str) -> Option<String> {
        if let Some(m) = self.library.mods.iter().find(|m| m.plugins.iter().any(|p| p.eq_ignore_ascii_case(file))) {
            return Some(m.name.clone());
        }
        self.library.mods.iter().find_map(|m| {
            walkdir::WalkDir::new(self.paths.mod_dir(&m.id))
                .into_iter()
                .flatten()
                .any(|e| e.file_type().is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case(file))
                .then(|| m.name.clone())
        })
    }

    /// Whether crashes will leave a report: Crash Logger (or Trainwreck) is in the game folder.
    fn crash_logger_installed(&self) -> bool {
        self.game.as_ref().is_some_and(|g| crash::logger_installed(&g.data_dir()))
    }

    fn dismiss_crash(&mut self) {
        if let Some(c) = self.crash.take() {
            self.settings.crash_seen = c.report.time;
            self.settings.save(&self.paths);
        }
        self.show_crash = false;
    }

    /// Asks Nexus for the current version of every mod installed from there.
    fn check_updates(&mut self, ctx: &egui::Context) {
        let Some(client) = self.nexus.clone() else { return };
        let ids: HashSet<u64> = self.library.mods.iter().filter_map(|m| m.nexus.as_ref().map(|n| n.mod_id)).collect();
        if ids.is_empty() {
            return;
        }
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let latest: Vec<(u64, String)> = ids
                .into_iter()
                .filter_map(|id| client.mod_info(id).ok().map(|i| (id, i.version)))
                .collect();
            let _ = tx.send(Msg::UpdatesChecked(latest));
            ctx.request_repaint();
        });
    }

    /// The newer version waiting on Nexus for this mod, if any.
    fn update_for(&self, m: &Mod) -> Option<&str> {
        let n = m.nexus.as_ref()?;
        let latest = self.latest_versions.get(&n.mod_id)?;
        (!latest.is_empty() && *latest != n.version).then_some(latest.as_str())
    }

    /// Library entry (if any) installed from this Nexus mod.
    fn installed_from(&self, mod_id: u64) -> Option<&Mod> {
        self.library.mods.iter().find(|m| m.nexus.as_ref().is_some_and(|n| n.mod_id == mod_id))
    }

    fn pick_mod_files(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title("Choose mod archives")
            .add_filter("Mods", &["zip", "7z", "rar", "esp", "esm", "esl", "bsa", "dll", "ini"])
            .add_filter("Mod archives", &["zip", "7z", "rar"])
            .pick_files();
        for f in files.unwrap_or_default() {
            self.queue_install(f);
        }
    }

    fn pick_game(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .set_title("Find SkyrimSE.exe")
            .add_filter("Skyrim", &["exe"])
            .pick_file();
        if let Some(p) = picked {
            match Game::from_user_path(&p) {
                Some(g) => {
                    self.plugins = game::scan_plugins(&g);
                    self.settings.game_path = Some(g.root.clone());
                    self.settings.save(&self.paths);
                    self.game = Some(g);
                    self.refresh_runtimes();
                    self.refresh_game_version();
                    self.needs_apply = !self.library.mods.is_empty();
                    self.toast(ctx, ToastKind::Success, "Found Skyrim Special Edition");
                }
                None => self.toast(
                    ctx,
                    ToastKind::Error,
                    "That folder doesn't contain SkyrimSE.exe",
                ),
            }
        }
    }

    fn save_plugins(&mut self, ctx: &egui::Context) {
        game::refresh_missing_masters(&mut self.plugins);
        if let Err(e) = game::write_plugins_txt(&self.plugins) {
            self.toast(ctx, ToastKind::Error, format!("Couldn't save load order: {e}"));
        }
    }

    fn play(&mut self, ctx: &egui::Context) {
        if self.busy.is_some() || self.needs_apply {
            self.toast(ctx, ToastKind::Error, "One moment — still applying your mods");
            return;
        }
        let Some(g) = &self.game else { return };
        match g.launch() {
            Ok(()) => self.toast(ctx, ToastKind::Success, "Launching Skyrim…"),
            Err(e) => self.toast(ctx, ToastKind::Error, e.to_string()),
        }
    }

    fn open_folder(path: &std::path::Path) {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }

    // -----------------------------------------------------------------------
    // Layout
    // -----------------------------------------------------------------------

    fn sidebar(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(w::icon(icon::GAME, 18.0, p.accent));
            ui.add_space(2.0);
            ui.label(RichText::new("Skyrim Mods").font(egui::FontId::new(16.0, theme::bold())));
        });
        ui.add_space(18.0);

        let items = [
            (Page::Library, icon::LIBRARY, "Library", self.library.mods.len()),
            (Page::Browse, icon::SEARCH, "Browse", 0),
            (Page::Plugins, icon::LIST, "Load Order", self.plugins.iter().filter(|p| p.enabled && !p.core).count()),
            (Page::Settings, icon::SETTINGS, "Settings", 0),
        ];
        for (page, glyph, label, count) in items {
            if self.sidebar_item(ui, p, self.page == page, glyph, label, count).clicked() {
                self.page = page;
                self.search.clear();
                if page == Page::Browse && self.browse.query.is_none() {
                    self.browse.typed.clear();
                    self.run_search(ui.ctx(), 0);
                }
            }
        }

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let (color, text) = match (&self.nexus_user, self.connecting) {
                    (Some(u), _) => (p.green, if u.is_premium { format!("Nexus Premium · {}", u.name) } else { format!("Nexus · {}", u.name) }),
                    (None, true) => (p.orange, "Connecting to Nexus…".to_string()),
                    (None, false) => (p.text_tertiary, "Nexus not connected".to_string()),
                };
                w::dot(ui, color);
                ui.add(egui::Label::new(w::secondary(&text, p)).truncate());
            });
            match &self.game {
                Some(g) => {
                    let skse = g.skse().is_some();
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        w::dot(ui, if skse { p.green } else { p.text_tertiary });
                        ui.label(w::secondary(if skse { "SKSE ready" } else { "SKSE not installed" }, p));
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        w::dot(ui, p.green);
                        ui.label(w::secondary("Skyrim Special Edition", p));
                    });
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        w::dot(ui, p.red);
                        ui.label(w::secondary("Skyrim not found", p));
                    });
                }
            }
            if let Some(b) = self.busy.clone() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    w::spinner(ui, p.accent);
                    let text = match b {
                        Busy::Installing(name) => format!("Installing {name}"),
                        Busy::Applying => "Applying mods…".to_string(),
                        Busy::Restoring => "Restoring…".to_string(),
                    };
                    ui.add(egui::Label::new(w::secondary(&text, p)).truncate());
                });
            }
            if !self.downloads.is_empty() {
                ui.add_space(8.0);
                let mut cancel = None;
                // Bottom-up layouts only reserve one line per row, so size the block up front.
                // Rows: 28 name line + 4 bar + 18 size text, 4 apart, 8 between rows.
                const ROW_H: f32 = 58.0;
                const GAP: f32 = 8.0;
                let n = self.downloads.len() as f32;
                let block = vec2(ui.available_width(), ROW_H * n + GAP * (n - 1.0));
                ui.allocate_ui_with_layout(block, Layout::top_down(Align::Min), |ui| {
                    ui.set_width(block.x);
                    ui.spacing_mut().item_spacing.y = GAP;
                    let row = vec2(block.x - 16.0, ROW_H);
                    for d in self.downloads.iter().rev() {
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.allocate_ui_with_layout(row, Layout::top_down(Align::Min), |ui| {
                                ui.set_width(row.x);
                                ui.spacing_mut().item_spacing.y = 4.0;
                                // Lay the cancel button out first so the name truncates around it.
                                ui.allocate_ui_with_layout(vec2(row.x, 28.0), Layout::right_to_left(Align::Center), |ui| {
                                    if w::icon_button(ui, icon::CLOSE, "Cancel download").clicked() {
                                        cancel = Some(d.id);
                                    }
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.add(egui::Label::new(RichText::new(&d.name).size(12.5).color(p.text)).truncate());
                                    });
                                });
                                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), egui::Sense::hover());
                                ui.painter().rect_filled(rect, 2.0, p.fill_tertiary);
                                match d.progress.fraction() {
                                    Some(f) => {
                                        let mut bar = rect;
                                        bar.set_width(rect.width() * f.clamp(0.0, 1.0));
                                        ui.painter().rect_filled(bar, 2.0, p.accent);
                                    }
                                    None => {
                                        // Size unknown: sweep a short segment back and forth.
                                        let t = (ui.input(|i| i.time) * 0.8).fract() as f32;
                                        let x = rect.left() + (rect.width() - 40.0) * (1.0 - (t * 2.0 - 1.0).abs());
                                        ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(x, rect.top()), vec2(40.0, 4.0)), 2.0, p.accent);
                                    }
                                }
                                let done = d.progress.done.load(std::sync::atomic::Ordering::Relaxed);
                                let total = d.progress.total.load(std::sync::atomic::Ordering::Relaxed);
                                let text = if total > 0 {
                                    format!("{} of {}", mods::format_size(done), mods::format_size(total))
                                } else {
                                    mods::format_size(done)
                                };
                                ui.label(w::secondary(&text, p));
                            });
                        });
                    }
                });
                if let Some(id) = cancel
                    && let Some(d) = self.downloads.iter().find(|d| d.id == id)
                {
                    d.progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
        });
    }

    fn sidebar_item(&self, ui: &mut Ui, p: &Palette, selected: bool, glyph: &str, label: &str, count: usize) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), egui::Sense::click());
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 8.0, p.fill_tertiary);
        } else if resp.hovered() {
            painter.rect_filled(rect, 8.0, p.fill_secondary);
        }
        painter.text(
            egui::pos2(rect.left() + 20.0, rect.center().y),
            Align2::CENTER_CENTER,
            glyph,
            egui::FontId::new(15.0, theme::icons()),
            if selected { p.accent } else { p.text_secondary },
        );
        painter.text(
            egui::pos2(rect.left() + 38.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            egui::FontId::new(14.0, if selected { theme::semibold() } else { egui::FontFamily::Proportional }),
            p.text,
        );
        if count > 0 {
            painter.text(
                egui::pos2(rect.right() - 12.0, rect.center().y),
                Align2::RIGHT_CENTER,
                count.to_string(),
                egui::FontId::new(12.5, egui::FontFamily::Proportional),
                p.text_tertiary,
            );
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    fn header(&mut self, ui: &mut Ui, p: &Palette, title: &str, subtitle: &str) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(w::heading(title));
                ui.label(w::secondary(subtitle, p));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let can_play = self.game.is_some() && self.busy.is_none() && !self.needs_apply;
                ui.add_enabled_ui(can_play, |ui| {
                    if w::pill(ui, Some(icon::PLAY), "Play", PillStyle::Primary).clicked() {
                        self.play(ui.ctx());
                    }
                });
                if self.page != Page::Settings {
                    ui.add_space(4.0);
                    if w::pill(ui, Some(icon::ADD), "Add Mod", PillStyle::Secondary).clicked() {
                        self.pick_mod_files();
                    }
                    ui.add_space(4.0);
                    w::search_field(ui, &mut self.search, 220.0);
                }
            });
        });
        ui.add_space(20.0);
    }

    // -----------------------------------------------------------------------
    // Pages
    // -----------------------------------------------------------------------

    /// Whether this row is the one being carried right now.
    fn lifted(&self, page: Page, index: usize) -> bool {
        self.drag.is_some_and(|d| d.page == page && d.from == index && d.decided)
    }

    /// Keeps a drag going over the rows drawn this frame (`rows` = index in the full list
    /// and where each was drawn, limited to rows a drop may land before). Shows where the
    /// row would go, scrolls the list when the pointer nears its edge, and returns
    /// `(from, before)` once it's dropped somewhere new.
    fn drag_update(&mut self, ui: &mut Ui, page: Page, rows: &[(usize, egui::Rect)]) -> Option<(usize, usize)> {
        let mut drag = self.drag?;
        if drag.page != page {
            self.drag = None;
            return None;
        }
        let (pointer, down, moved, escape) = ui.input(|i| {
            (i.pointer.latest_pos(), i.pointer.primary_down(), i.pointer.is_decidedly_dragging(), i.key_pressed(egui::Key::Escape))
        });
        if moved && !drag.decided {
            drag.decided = true;
            self.drag = Some(drag);
        }
        // Let go without moving: that was a click on whatever is in the row.
        if escape || !drag.decided {
            if escape || !down {
                self.drag = None;
            }
            return None;
        }
        let Some(pos) = pointer else {
            if !down {
                self.drag = None;
            }
            return None;
        };
        let from = drag.from;
        let slot = drop_slot(rows, pos.y);
        let before = match rows.get(slot) {
            Some((i, _)) => *i,
            None => rows.last().map_or(from, |(i, _)| i + 1),
        };
        let moves = before != from && before != from + 1;
        if !down {
            self.drag = None;
            return moves.then_some((from, before));
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
        if moves && let Some((_, first)) = rows.first() {
            let y = match rows.get(slot) {
                Some((_, r)) => r.top(),
                None => rows.last().map_or(first.top(), |(_, r)| r.bottom()),
            };
            w::drop_marker(ui, first.x_range(), y);
        }
        // Keep the list moving when the row is carried past its edge.
        let clip = ui.clip_rect();
        let edge = 48.0;
        let step = if pos.y < clip.top() + edge {
            (clip.top() + edge - pos.y) * 0.25
        } else if pos.y > clip.bottom() - edge {
            (clip.bottom() - edge - pos.y) * 0.25
        } else {
            0.0
        };
        if step != 0.0 {
            ui.scroll_with_delta_animation(vec2(0.0, step), egui::style::ScrollAnimation::none());
        }
        None
    }

    fn page_library(&mut self, ui: &mut Ui, p: &Palette) {
        let total = self.library.mods.len();
        let enabled = self.library.mods.iter().filter(|m| m.enabled).count();
        let subtitle = if total == 0 {
            "Your mods live here".to_string()
        } else {
            format!("{total} mods · {enabled} enabled")
        };
        self.header(ui, p, "Library", &subtitle);

        if self.game.is_none() {
            self.no_game_card(ui, p);
            return;
        }
        if total == 0 {
            self.empty_library(ui, p);
            return;
        }

        if self.apply_failed {
            self.notice(ui, p, p.red, "Your last change couldn't be applied. Fix the problem in Settings, then try again.", Some("Try again"), |s| s.needs_apply = true);
        }

        let filter = self.search.trim().to_ascii_lowercase();
        let ids: Vec<String> = self
            .library
            .mods
            .iter()
            .filter(|m| filter.is_empty() || m.name.to_ascii_lowercase().contains(&filter))
            .map(|m| m.id.clone())
            .collect();

        if ids.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(w::secondary("No mods match your search.", p));
            });
            return;
        }

        let mut move_req: Option<(String, isize)> = None;
        let mut toggled = false;
        // Where each row was drawn, for dragging one to a new spot.
        let mut rows: Vec<(usize, egui::Rect)> = Vec::new();
        let mut open_req: Option<PathBuf> = None;
        let mut update_req: Option<(String, u64, String, String)> = None;
        let mut url_req: Option<String> = None;
        let mut find_req: Option<String> = None;
        let mut turn_on_req: Option<String> = None;
        let mut reqs_req: Option<(u64, String)> = None;
        let mut install_address_library = false;
        let updates_available = self.library.mods.iter().filter(|m| self.update_for(m).is_some()).count();
        // What each mod is missing, worked out once per frame rather than per row.
        let missing: HashMap<String, Missing> = self
            .library
            .mods
            .iter()
            .filter(|m| m.enabled)
            .map(|m| (m.id.clone(), self.missing_for(m)))
            .filter(|(_, x)| !x.is_empty())
            .collect();
        let game = self.game_label();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if filter.is_empty() {
                self.crash_notices(ui, p);
                self.needs_notices(ui, p, &missing);
            }
            if updates_available > 0 && filter.is_empty() {
                let text = if updates_available == 1 {
                    "1 mod has a newer version on Nexus Mods.".to_string()
                } else {
                    format!("{updates_available} mods have newer versions on Nexus Mods.")
                };
                let ctx = ui.ctx().clone();
                self.notice(ui, p, p.accent, &text, Some("Update all"), move |s| {
                    let list: Vec<(String, u64, String, String)> = s
                        .library
                        .mods
                        .iter()
                        .filter_map(|m| {
                            let n = m.nexus.as_ref()?;
                            s.update_for(m).map(|v| (m.id.clone(), n.mod_id, m.name.clone(), v.to_string()))
                        })
                        .collect();
                    for (id, mod_id, name, version) in list {
                        s.install_from_nexus(&ctx, mod_id, &name, &version, Some(id), false);
                    }
                });
            }
            w::grouped(ui, |ui| {
                let count = ids.len();
                for (i, id) in ids.iter().enumerate() {
                    let Some(idx) = self.library.mods.iter().position(|m| &m.id == id) else { continue };
                    let is_first = idx == 0;
                    let is_last = idx + 1 == self.library.mods.len();
                    let mod_dir = self.paths.mod_dir(id);
                    let update = self.update_for(&self.library.mods[idx]).map(str::to_string);
                    let downloading = self.library.mods[idx]
                        .nexus
                        .as_ref()
                        .is_some_and(|n| self.downloads.iter().any(|d| d.mod_id == n.mod_id) || self.fetching_files.contains(&n.mod_id));
                    let thumb = self.library.mods[idx].nexus.as_ref().and_then(|n| self.thumbs.get(&n.mod_id).cloned().flatten());
                    let needs = self.needs.get(id).map(|n| self.needs_text(n)).unwrap_or_default();
                    // A mod that's switched off isn't missing anything, but it's still worth
                    // knowing it was made for another game version before switching it on.
                    let lacking = missing.get(id).cloned().unwrap_or_else(|| Missing {
                        outdated: self.outdated_for(&self.library.mods[idx]),
                        ..Missing::default()
                    });
                    let lifted = self.lifted(Page::Library, idx);
                    let m = &mut self.library.mods[idx];
                    let (resp, _) = w::row_drag(ui, &m.id, lifted, |ui| {
                        if w::toggle(ui, &mut m.enabled).changed() {
                            toggled = true;
                        }
                        ui.add_space(8.0);
                        // Mods added from a file have no picture; show a folder instead.
                        Self::thumbnail(ui, p, thumb.as_ref(), m.nexus.is_none().then_some(icon::FOLDER));
                        ui.add_space(10.0);
                        // Trailing controls go down first so the text can truncate around them.
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let more = w::icon_button(ui, icon::MORE, "More");
                            egui::Popup::menu(&more).width(240.0).show(|ui| {
                                if let Some(v) = &update
                                    && let Some(n) = &m.nexus
                                    && ui.add_enabled(!downloading, egui::Button::new(format!("Update to {v}"))).clicked()
                                {
                                    update_req = Some((id.clone(), n.mod_id, m.name.clone(), v.clone()));
                                }
                                if ui.button("Show files").clicked() {
                                    open_req = Some(mod_dir.clone());
                                }
                                if let Some(n) = &m.nexus {
                                    if ui.button("Requirements").clicked() {
                                        reqs_req = Some((n.mod_id, m.name.clone()));
                                    }
                                    if ui.button("Open on Nexus Mods").clicked() {
                                        url_req = Some(format!("{NEXUS_URL}{}", n.mod_id));
                                    }
                                }
                                // Shortcuts to whatever this mod is missing.
                                if !lacking.is_empty() {
                                    ui.separator();
                                    if lacking.skse && ui.button("Get SKSE").clicked() {
                                        url_req = Some(SKSE_URL.to_string());
                                    }
                                    if lacking.address_library && ui.button("Install Address Library").clicked() {
                                        install_address_library = true;
                                    }
                                    for master in &lacking.masters {
                                        if ui.button(format!("Find {} on Nexus", needs::display_name(master))).clicked() {
                                            find_req = Some(needs::display_name(master));
                                        }
                                    }
                                    for name in &lacking.off {
                                        if ui.button(format!("Turn on {name}")).clicked() {
                                            turn_on_req = Some(name.clone());
                                        }
                                    }
                                    if !lacking.outdated.is_empty() && ui.button("Look for a newer version").clicked() {
                                        match &m.nexus {
                                            Some(n) => url_req = Some(format!("{NEXUS_URL}{}", n.mod_id)),
                                            None => find_req = Some(m.name.clone()),
                                        }
                                    }
                                }
                                ui.add_enabled_ui(!is_first, |ui| {
                                    if ui.button("Move up (lower priority)").clicked() {
                                        move_req = Some((id.clone(), -1));
                                    }
                                });
                                ui.add_enabled_ui(!is_last, |ui| {
                                    if ui.button("Move down (higher priority)").clicked() {
                                        move_req = Some((id.clone(), 1));
                                    }
                                });
                                ui.separator();
                                if ui.button(RichText::new("Remove…").color(p.red)).clicked() {
                                    self.confirm_remove = Some(id.clone());
                                }
                            });
                            if !m.enabled {
                                w::badge(ui, "Off", p.text_tertiary);
                            }
                            if lacking.lacks_anything() {
                                ui.label(w::icon(icon::WARNING, 13.0, p.orange)).on_hover_text("Something this mod needs isn't installed");
                            } else if !lacking.outdated.is_empty() {
                                ui.label(w::icon(icon::WARNING, 13.0, p.orange)).on_hover_text("This mod was made for another version of Skyrim");
                            }
                            if downloading {
                                w::spinner(ui, p.accent);
                            } else if let Some(v) = &update
                                && let Some(n) = &m.nexus
                            {
                                if w::pill(ui, Some(icon::DOWNLOAD), "Update", PillStyle::Secondary)
                                    .on_hover_text(format!("Version {v} is on Nexus Mods"))
                                    .clicked()
                                {
                                    update_req = Some((id.clone(), n.mod_id, m.name.clone(), v.clone()));
                                }
                            }
                            ui.add_space(8.0);
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.vertical(|ui| {
                                    ui.add(egui::Label::new(w::title(&m.name)).truncate());
                                    let mut meta = String::new();
                                    if let Some(n) = &m.nexus
                                        && !n.version.is_empty()
                                    {
                                        meta.push_str(&format!("v{} · ", n.version));
                                    }
                                    meta.push_str(&format!(
                                        "{} {} · {}",
                                        m.file_count,
                                        if m.file_count == 1 { "file" } else { "files" },
                                        mods::format_size(m.size_bytes)
                                    ));
                                    match m.plugins.len() {
                                        0 => {}
                                        1 => meta.push_str(&format!(" · {}", m.plugins[0])),
                                        n => meta.push_str(&format!(" · {n} plugins")),
                                    }
                                    if !m.root_files.is_empty() {
                                        meta.push_str(" · Game folder: ");
                                        meta.push_str(&m.root_files.join(", "));
                                    }
                                    ui.add(egui::Label::new(w::secondary(&meta, p)).truncate());
                                    // What it depends on — in orange when something isn't there yet.
                                    if lacking.lacks_anything() {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(format!("Missing {}", lacking.list())).size(12.5).color(p.orange),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(format!("Needs {needs}"));
                                    } else if let Some(text) = lacking.outdated_text(&game) {
                                        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.orange)).truncate())
                                            .on_hover_text(outdated_hint(&lacking, &game));
                                    } else if !needs.is_empty() {
                                        ui.add(
                                            egui::Label::new(RichText::new(format!("Needs {needs}")).size(12.5).color(p.text_tertiary))
                                                .truncate(),
                                        );
                                    }
                                });
                            });
                        });
                    });
                    if resp.drag_started() && self.drag.is_none() {
                        self.drag = Some(Drag { page: Page::Library, from: idx, decided: false });
                    }
                    rows.push((idx, resp.rect));
                    if i + 1 < count {
                        w::hairline(ui);
                    }
                }
            });
            if let Some((from, before)) = self.drag_update(ui, Page::Library, &rows) {
                move_item(&mut self.library.mods, from, before);
                toggled = true;
            }

            ui.add_space(12.0);
            let mut hint = String::from("Drag a mod to change its place. Mods lower in the list win when files overlap.");
            if let Some(r) = &self.last_report
                && r.conflicts > 0
            {
                hint.push_str(&format!(
                    " {} overlapping {} right now.",
                    r.conflicts,
                    if r.conflicts == 1 { "file" } else { "files" }
                ));
            }
            ui.label(w::secondary(&hint, p));
        });

        if let Some(dir) = open_req {
            Self::open_folder(&dir);
        }
        if let Some(url) = url_req {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
        if let Some((id, mod_id, name, version)) = update_req {
            self.install_from_nexus(ui.ctx(), mod_id, &name, &version, Some(id), false);
        }
        if install_address_library {
            self.install_address_library(ui.ctx());
        }
        if let Some(name) = find_req {
            self.find_on_nexus(name);
        }
        if let Some(name) = turn_on_req {
            self.enable_mods(&[name]);
        }
        if let Some((id, name)) = reqs_req {
            self.request_requirements(ui.ctx(), vec![id]);
            self.req_sheet = Some((id, name));
        }
        if let Some((id, delta)) = move_req
            && let Some(i) = self.library.mods.iter().position(|m| m.id == id)
        {
            let j = (i as isize + delta).clamp(0, self.library.mods.len() as isize - 1) as usize;
            self.library.mods.swap(i, j);
            toggled = true;
        }
        if toggled {
            let _ = self.library.save(&self.paths);
            self.needs_apply = true;
        }
    }

    // -----------------------------------------------------------------------
    // Browse (Nexus Mods)
    // -----------------------------------------------------------------------

    /// Rounded text field for the API key. Returns true when Enter was pressed.
    fn key_field(&mut self, ui: &mut Ui, p: &Palette, width: f32) -> bool {
        let height = 32.0;
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
        ui.painter().rect_filled(rect, height / 2.0, p.fill_secondary);
        let edit_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 14.0, rect.top() + 6.0),
            egui::pos2(rect.right() - 14.0, rect.bottom() - 6.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
        let resp = child.add(
            egui::TextEdit::singleline(&mut self.key_input)
                .password(true)
                .hint_text(RichText::new("Paste your API key").color(p.text_tertiary))
                .frame(Frame::NONE)
                .desired_width(edit_rect.width()),
        );
        resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
    }

    /// Key entry + "Get key" + Connect, used on the Browse page and in Settings.
    fn connect_controls(&mut self, ui: &mut Ui, p: &Palette, width: f32) {
        ui.horizontal(|ui| {
            let submit = self.key_field(ui, p, width);
            ui.add_space(4.0);
            ui.add_enabled_ui(!self.connecting && !self.key_input.trim().is_empty(), |ui| {
                if w::pill(ui, None, "Connect", PillStyle::Primary).clicked() || submit {
                    let key = self.key_input.clone();
                    self.connect(ui.ctx(), &key, true);
                }
            });
            if self.connecting {
                w::spinner(ui, p.accent);
            }
            if w::pill(ui, Some(icon::OPEN), "Get key", PillStyle::Ghost)
                .on_hover_text("Opens your Nexus Mods account page. Copy the Personal API Key at the bottom.")
                .clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(nexus::API_KEY_URL));
            }
        });
    }

    fn connect_card(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(520.0);
            w::card(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.label(w::icon(icon::DOWNLOAD, 40.0, p.accent));
                    ui.add_space(12.0);
                    ui.label(RichText::new("Connect Nexus Mods").font(egui::FontId::new(20.0, theme::bold())));
                    ui.add_space(6.0);
                    ui.label(w::secondary(
                        "Search and install mods right here. Sign in on nexusmods.com, open your account's API Access tab and paste the Personal API Key below.",
                        p,
                    ));
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        // field + Connect + Get key, centred in the card
                        ui.add_space((ui.available_width() - 520.0).max(0.0) / 2.0);
                        self.connect_controls(ui, p, 260.0);
                    });
                    ui.add_space(12.0);
                });
            });
        });
    }

    fn page_browse(&mut self, ui: &mut Ui, p: &Palette) {
        let subtitle = match &self.nexus_user {
            Some(u) if u.is_premium => "Nexus Mods · downloads go straight into your library",
            Some(_) => "Nexus Mods · free account: downloads open on the website",
            None => "Search Nexus Mods for Skyrim Special Edition",
        };
        self.header(ui, p, "Browse", subtitle);

        if self.nexus_user.is_none() {
            self.connect_card(ui, p);
            return;
        }
        let game = self.game_label();

        // Debounced search: fire a query 350 ms after the last keystroke.
        let now = ui.input(|i| i.time);
        if self.search != self.browse.typed {
            self.browse.typed = self.search.clone();
            self.browse.last_typed = now;
        }
        let want = self.browse.typed.trim().to_string();
        if self.browse.query.as_deref() != Some(want.as_str()) {
            if now - self.browse.last_typed > 0.35 {
                self.run_search(ui.ctx(), 0);
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
            }
        }

        let heading = match self.browse.query.as_deref() {
            Some("") | None => "Most endorsed".to_string(),
            Some(q) => match self.browse.total {
                0 if self.browse.loading => format!("Searching for “{q}”"),
                0 => format!("Nothing found for “{q}”"),
                1 => "1 result".to_string(),
                n => format!("{} results", nexus::compact_count(n)),
            },
        };
        ui.horizontal(|ui| {
            ui.label(w::section(&heading.to_ascii_uppercase(), p));
            if self.browse.loading {
                ui.add_space(4.0);
                w::spinner(ui, p.accent);
            }
        });
        ui.add_space(6.0);

        if let Some(e) = self.browse.error.clone() {
            self.notice(ui, p, p.red, &e, Some("Try again"), |s| {
                s.browse.query = None;
            });
        }

        let mut install: Option<(u64, String, String, bool)> = None;
        let mut open: Option<u64> = None;
        let mut show_reqs: Option<(u64, String)> = None;
        let mut more = false;
        let premium = self.is_premium();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if !self.browse.results.is_empty() {
                w::grouped(ui, |ui| {
                    let count = self.browse.results.len();
                    for i in 0..count {
                        let m = &self.browse.results[i];
                        let installed = self.installed_from(m.mod_id);
                        let update = installed.and_then(|im| self.update_for(im)).map(str::to_string);
                        let busy = self.downloads.iter().any(|d| d.mod_id == m.mod_id) || self.fetching_files.contains(&m.mod_id);
                        let thumb = self.thumbs.get(&m.mod_id).cloned().flatten();
                        // What it needs: the Nexus page's list when we have it, otherwise (once
                        // it's installed) whatever its own files say.
                        let lacking = installed.filter(|im| im.enabled).map(|im| self.missing_for(im)).unwrap_or_default();
                        let (needs, needs_lacking) = match self.requirements.get(&m.mod_id) {
                            Some(Some(reqs)) if !reqs.is_empty() => self.requirements_text(reqs),
                            _ => (installed.and_then(|im| self.needs.get(&im.id)).map(|n| self.needs_text(n)).unwrap_or_default(), false),
                        };
                        w::row(ui, |ui| {
                            Self::thumbnail(ui, p, thumb.as_ref(), None);
                            ui.add_space(10.0);
                            // Trailing controls go down first so the text wraps around them.
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if busy {
                                    w::spinner(ui, p.accent);
                                } else if let Some(im) = installed {
                                    if let Some(v) = &update {
                                        if w::pill(ui, Some(icon::DOWNLOAD), "Update", PillStyle::Primary)
                                            .on_hover_text(format!("Update to {v}"))
                                            .clicked()
                                        {
                                            install = Some((m.mod_id, m.name.clone(), v.clone(), false));
                                        }
                                    } else if premium
                                        && w::pill(ui, None, "Files…", PillStyle::Ghost)
                                            .on_hover_text("Install another file from this mod")
                                            .clicked()
                                    {
                                        install = Some((m.mod_id, m.name.clone(), m.version.clone(), true));
                                    }
                                    let _ = im;
                                    w::badge(ui, "Installed", p.green);
                                } else if premium {
                                    if w::pill(ui, Some(icon::DOWNLOAD), "Install", PillStyle::Primary).clicked() {
                                        install = Some((m.mod_id, m.name.clone(), m.version.clone(), false));
                                    }
                                } else if w::pill(ui, Some(icon::OPEN), "Get", PillStyle::Secondary)
                                    .on_hover_text("Opens the mod on Nexus Mods — use “Mod Manager Download” there")
                                    .clicked()
                                {
                                    open = Some(m.mod_id);
                                }
                                if w::icon_button(ui, icon::OPEN, "Open on Nexus Mods").clicked() {
                                    open = Some(m.mod_id);
                                }
                                if w::pill(ui, None, "Requirements", PillStyle::Ghost)
                                    .on_hover_text("What this mod needs before it works")
                                    .clicked()
                                {
                                    show_reqs = Some((m.mod_id, m.name.clone()));
                                }
                                ui.add_space(8.0);
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.vertical(|ui| {
                                        ui.add(egui::Label::new(w::title(&m.name)).truncate());
                                        let mut meta = Vec::new();
                                        if !m.author.is_empty() {
                                            meta.push(format!("by {}", m.author));
                                        }
                                        if !m.version.is_empty() {
                                            meta.push(format!("v{}", m.version));
                                        }
                                        meta.push(format!("{} downloads", nexus::compact_count(m.downloads)));
                                        meta.push(format!("{} endorsements", nexus::compact_count(m.endorsements)));
                                        ui.add(egui::Label::new(w::secondary(&meta.join(" · "), p)).truncate());
                                        if !m.summary.is_empty() {
                                            ui.label(RichText::new(&m.summary).size(12.5).color(p.text_secondary));
                                        }
                                        if lacking.lacks_anything() {
                                            ui.add(egui::Label::new(RichText::new(format!("Missing {}", lacking.list())).size(12.5).color(p.orange)).truncate());
                                        } else if let Some(text) = lacking.outdated_text(&game) {
                                            ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.orange)).truncate())
                                                .on_hover_text(outdated_hint(&lacking, &game));
                                        } else if !needs.is_empty() {
                                            let color = if needs_lacking { p.orange } else { p.text_tertiary };
                                            ui.add(egui::Label::new(RichText::new(format!("Needs {needs}")).size(12.5).color(color)).truncate());
                                        }
                                    });
                                });
                            });
                        });
                        if i + 1 < count {
                            w::hairline(ui);
                        }
                    }
                });
                ui.add_space(12.0);
                if (self.browse.results.len() as u64) < self.browse.total {
                    ui.vertical_centered(|ui| {
                        ui.add_enabled_ui(!self.browse.loading, |ui| {
                            if w::pill(ui, None, "Show more", PillStyle::Secondary).clicked() {
                                more = true;
                            }
                        });
                    });
                    ui.add_space(12.0);
                }
            } else if !self.browse.loading && self.browse.error.is_none() && self.browse.query.is_some() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(w::secondary("Try a different name, or browse the site.", p));
                    ui.add_space(8.0);
                    if w::pill(ui, Some(icon::OPEN), "Open Nexus Mods", PillStyle::Ghost).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(NEXUS_URL));
                    }
                });
            }
        });

        if let Some((mod_id, name, version, force)) = install {
            let replaces = if force {
                None
            } else {
                self.installed_from(mod_id).filter(|m| self.update_for(m).is_some()).map(|m| m.id.clone())
            };
            self.install_from_nexus(ui.ctx(), mod_id, &name, &version, replaces, force);
        }
        if let Some(id) = open {
            ui.ctx().open_url(egui::OpenUrl::new_tab(format!("{NEXUS_URL}{id}")));
        }
        if let Some((id, name)) = show_reqs {
            self.request_requirements(ui.ctx(), vec![id]);
            self.req_sheet = Some((id, name));
        }
        if more {
            let offset = self.browse.results.len() as u64;
            self.run_search(ui.ctx(), offset);
        }
    }

    /// 16:9 preview, centre-cropped, with a soft placeholder while it loads. Pass an
    /// `icon` for the placeholder when there will never be a picture (a local file).
    fn thumbnail(ui: &mut Ui, p: &Palette, tex: Option<&egui::TextureHandle>, icon: Option<&str>) {
        let size = vec2(88.0, 49.5);
        match tex {
            Some(t) => {
                let [w, h] = t.size();
                let (w, h) = (w as f32, h as f32);
                let target = size.x / size.y;
                let uv = if w / h > target {
                    let f = target * h / w;
                    egui::Rect::from_min_max(egui::pos2((1.0 - f) / 2.0, 0.0), egui::pos2((1.0 + f) / 2.0, 1.0))
                } else {
                    let f = (w / h) / target;
                    egui::Rect::from_min_max(egui::pos2(0.0, (1.0 - f) / 2.0), egui::pos2(1.0, (1.0 + f) / 2.0))
                };
                // The uv crop already fixes the aspect, so fill the slot exactly.
                ui.add(egui::Image::from_texture(t).fit_to_exact_size(size).maintain_aspect_ratio(false).uv(uv).corner_radius(6.0));
            }
            None => {
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                ui.painter().rect_filled(rect, 6.0, p.fill_secondary);
                if let Some(icon) = icon {
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        icon,
                        egui::FontId::new(18.0, theme::icons()),
                        p.text_tertiary,
                    );
                }
            }
        }
    }

    fn page_plugins(&mut self, ui: &mut Ui, p: &Palette) {
        let active = self.plugins.iter().filter(|p| p.enabled).count();
        let full = self
            .plugins
            .iter()
            .filter(|p| p.enabled && p.kind != PluginKind::Light)
            .count();
        let light = self
            .plugins
            .iter()
            .filter(|p| p.enabled && p.kind == PluginKind::Light)
            .count();
        self.header(ui, p, "Load Order", &format!("{active} plugins active"));

        if self.game.is_none() {
            self.no_game_card(ui, p);
            return;
        }

        ui.horizontal(|ui| {
            Self::stat(ui, p, "Full plugins", &format!("{full}"), &format!("of {MAX_FULL_PLUGINS}"), full >= MAX_FULL_PLUGINS);
            Self::stat(ui, p, "Light plugins", &format!("{light}"), &format!("of {MAX_LIGHT_PLUGINS}"), light >= MAX_LIGHT_PLUGINS);
            let missing = self.plugins.iter().filter(|p| p.enabled && !p.missing_masters.is_empty()).count();
            Self::stat(ui, p, "Problems", &format!("{missing}"), if missing == 0 { "all good" } else { "missing masters" }, missing > 0);
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if w::pill(ui, Some(icon::REFRESH), "Sort", PillStyle::Secondary)
                    .on_hover_text("Put master files first, keeping everything else in order")
                    .clicked()
                {
                    let mut i = 0;
                    self.plugins.sort_by_key(|p| {
                        i += 1;
                        (!p.core, p.kind != PluginKind::Master, i)
                    });
                    self.save_plugins(ui.ctx());
                }
            });
        });
        ui.add_space(16.0);

        let filter = self.search.trim().to_ascii_lowercase();
        let mut changed = false;
        let mut move_req: Option<(usize, isize)> = None;
        // Where each of the user's rows was drawn, for dragging one to a new spot.
        let mut rows: Vec<(usize, egui::Rect)> = Vec::new();
        let count = self.plugins.len();

        let core_count = self.plugins.iter().filter(|p| p.core).count();
        let user_count = count - core_count;

        // Which mod each plugin came from: its name, and its picture if it has one.
        let owners: HashMap<String, (String, Option<egui::TextureHandle>, bool)> = self
            .library
            .mods
            .iter()
            .flat_map(|m| {
                let thumb = m.nexus.as_ref().and_then(|n| self.thumbs.get(&n.mod_id).cloned().flatten());
                m.plugins.iter().map(move |pl| (pl.to_ascii_lowercase(), (m.name.clone(), thumb.clone(), m.nexus.is_some())))
            })
            .collect();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            w::grouped(ui, |ui| {
                let mut shown = 0;
                // Built-in plugins are folded into one row so the user's own mods come first.
                if filter.is_empty() && core_count > 0 {
                    let (resp, _) = w::row_button(ui, |ui| {
                        w::locked_toggle(ui);
                        ui.add_space(8.0);
                        Self::thumbnail(ui, p, None, Some(icon::GAME));
                        ui.add_space(10.0);
                        ui.vertical(|ui| {
                            ui.label(w::title("Skyrim and its expansions"));
                            ui.label(w::secondary(
                                &format!("{core_count} built-in plugins · always loaded first"),
                                p,
                            ));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(8.0);
                            ui.label(w::icon(
                                if self.show_core { icon::CHEVRON_UP } else { icon::CHEVRON_DOWN },
                                12.0,
                                p.text_tertiary,
                            ));
                        });
                    });
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.show_core = !self.show_core;
                    }
                    shown += 1;
                }
                for i in 0..count {
                    let core = self.plugins[i].core;
                    let visible = if filter.is_empty() {
                        !core || self.show_core
                    } else {
                        self.plugins[i].name.to_ascii_lowercase().contains(&filter)
                    };
                    if !visible {
                        continue;
                    }
                    if shown > 0 {
                        w::hairline(ui);
                    }
                    shown += 1;
                    let (kind, name, missing, enabled, masters) = {
                        let pl = &self.plugins[i];
                        // Masters the game itself ships (including Creation Club ones) aren't worth a mention.
                        let masters: Vec<String> = pl
                            .masters
                            .iter()
                            .filter(|m| !game::is_core_plugin(m) && !matches!(self.provider_of(m), Provider::Game))
                            .cloned()
                            .collect();
                        (pl.kind, pl.name.clone(), pl.missing_masters.clone(), pl.enabled, masters)
                    };
                    // A missing master is switched off here, comes from a Library mod that's
                    // switched off, or isn't installed at all.
                    let missing_text = join_names(
                        &missing
                            .iter()
                            .map(|m| {
                                let present = self.plugins.iter().any(|p| p.name.eq_ignore_ascii_case(m));
                                match self.provider_of(m) {
                                    _ if present => format!("{} (turned off)", needs::display_name(m)),
                                    Provider::Mod(o) => format!("{} (from {}, which is turned off)", needs::display_name(m), o.name),
                                    _ => format!("{} (not installed)", needs::display_name(m)),
                                }
                            })
                            .collect::<Vec<_>>(),
                    );
                    let owner = owners.get(&name.to_ascii_lowercase());
                    let lifted = self.lifted(Page::Plugins, i);
                    let contents = |ui: &mut Ui| {
                        if core {
                            w::locked_toggle(ui).on_hover_text("Part of the game — always loaded");
                        } else if w::toggle(ui, &mut self.plugins[i].enabled).changed() {
                            changed = true;
                        }
                        ui.add_space(8.0);
                        // The picture of the mod this plugin belongs to; the game's own
                        // plugins get a controller, loose or file-installed ones a folder.
                        let (thumb, fallback) = match owner {
                            Some((_, t, from_nexus)) => (t.as_ref(), (!from_nexus).then_some(icon::FOLDER)),
                            None => (None, Some(if core { icon::GAME } else { icon::FOLDER })),
                        };
                        Self::thumbnail(ui, p, thumb, fallback);
                        ui.add_space(10.0);
                        // Trailing controls go down first so the text truncates around them.
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if !core {
                                ui.add_enabled_ui(i + 1 < count && !self.plugins[i + 1].core, |ui| {
                                    if w::icon_button(ui, icon::CHEVRON_DOWN, "Load later").clicked() {
                                        move_req = Some((i, 1));
                                    }
                                });
                                ui.add_enabled_ui(i > 0 && !self.plugins[i - 1].core, |ui| {
                                    if w::icon_button(ui, icon::CHEVRON_UP, "Load earlier").clicked() {
                                        move_req = Some((i, -1));
                                    }
                                });
                                ui.add_space(6.0);
                            }
                            let color = match kind {
                                PluginKind::Master => p.accent,
                                PluginKind::Light => p.green,
                                PluginKind::Normal => p.text_secondary,
                            };
                            w::badge(ui, kind.label(), color);
                            if !missing.is_empty() && enabled {
                                ui.label(w::icon(icon::WARNING, 13.0, p.orange));
                            }
                            ui.add_space(8.0);
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.vertical(|ui| {
                                    ui.add(egui::Label::new(w::title(&name)).truncate());
                                    if !missing.is_empty() && enabled {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(format!("Needs {missing_text}"))
                                                    .size(12.5)
                                                    .color(p.orange),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text("Skyrim crashes on launch when a plugin's master is missing");
                                    } else if core {
                                        ui.label(w::secondary("Built in", p));
                                    } else {
                                        let mut text = String::from(if enabled { "Loads with the game" } else { "Off" });
                                        // Skip the mod name when it just repeats the file name.
                                        let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
                                        if let Some((mod_name, _, _)) = owner
                                            && !mod_name.eq_ignore_ascii_case(stem)
                                        {
                                            text.push_str(" · ");
                                            text.push_str(mod_name);
                                        }
                                        if !masters.is_empty() {
                                            let names: Vec<String> = masters.iter().map(|m| needs::display_name(m)).collect();
                                            text.push_str(&format!(" · needs {}", join_names(&names)));
                                        }
                                        ui.add(egui::Label::new(w::secondary(&text, p)).truncate());
                                    }
                                });
                            });
                        });
                    };
                    // The game's own plugins stay put; only the user's can be dragged.
                    if core {
                        w::row(ui, contents);
                    } else {
                        let (resp, _) = w::row_drag(ui, &name, lifted, contents);
                        if resp.drag_started() && self.drag.is_none() {
                            self.drag = Some(Drag { page: Page::Plugins, from: i, decided: false });
                        }
                        rows.push((i, resp.rect));
                    }
                }
                if shown == 0 || (filter.is_empty() && user_count == 0) {
                    if shown > 0 {
                        w::hairline(ui);
                    }
                    Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(w::secondary(
                                if filter.is_empty() { "Mods with plugins will show up here once you install them." } else { "No plugins match your search." },
                                p,
                            ));
                        });
                    });
                }
            });
            if let Some((from, before)) = self.drag_update(ui, Page::Plugins, &rows) {
                move_item(&mut self.plugins, from, before);
                changed = true;
            }
            ui.add_space(12.0);
            ui.label(w::secondary(
                "Plugins load top to bottom — drag one to change when it loads. Light plugins (ESL) don't count toward the 254 limit.",
                p,
            ));
        });

        if let Some((i, d)) = move_req {
            let j = (i as isize + d).clamp(0, count as isize - 1) as usize;
            self.plugins.swap(i, j);
            changed = true;
        }
        if changed {
            self.save_plugins(ui.ctx());
        }
    }

    fn stat(ui: &mut Ui, p: &Palette, label: &str, value: &str, note: &str, warn: bool) {
        Frame::new()
            .fill(p.card)
            .stroke(egui::Stroke::new(1.0, p.stroke))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_min_width(140.0);
                ui.vertical(|ui| {
                    ui.label(w::section(label, p));
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(value)
                                .font(egui::FontId::new(22.0, theme::bold()))
                                .color(if warn { p.orange } else { p.text }),
                        );
                        ui.label(w::secondary(note, p));
                    });
                });
            });
    }

    /// Graphics card, driver, and which DLSS runtimes upscaler mods have put in the game folder.
    fn graphics_card(&mut self, ui: &mut Ui, p: &Palette) {
        w::grouped(ui, |ui| {
            w::row(ui, |ui| {
                ui.vertical(|ui| {
                    match &self.gpu {
                        Some(g) => {
                            ui.label(w::title(&g.name));
                            let mut line = format!("Driver {}", g.driver);
                            if g.rtx_generation().is_some_and(|g| g >= 50) {
                                line.push_str(match g.driver_supports_dlss5() {
                                    Some(true) => " · Ready for DLSS 5",
                                    Some(false) => " · Update to 595.79 or newer for DLSS 5",
                                    None => "",
                                });
                            }
                            ui.label(w::secondary(&line, p));
                        }
                        None => {
                            ui.label(w::title("Graphics card"));
                            ui.label(w::secondary("Couldn't read the display driver", p));
                        }
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(g) = &self.gpu
                        && g.vendor == gpu::Vendor::Nvidia
                    {
                        match g.rtx_generation() {
                            Some(series) if series >= 50 => w::badge(ui, "DLSS 5", p.green),
                            Some(series) if series >= 20 => w::badge(ui, "DLSS 4", p.accent),
                            _ => {}
                        }
                    }
                });
            });
            w::hairline(ui);
            w::row(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(w::title("DLSS in Skyrim"));
                    let text = if self.game.is_none() {
                        "Find Skyrim first".to_string()
                    } else if self.runtimes.is_empty() {
                        "Not installed — add an upscaler mod (Open Shaders, Community Shaders or \
                         Skyrim Upscaler) and its DLLs are placed where the game needs them."
                            .to_string()
                    } else {
                        self.runtimes
                            .iter()
                            .map(|r| match &r.version {
                                Some(v) => format!("{} {}", r.label, v),
                                None => r.label.to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join(" · ")
                    };
                    ui.label(w::secondary(&text, p));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.runtimes.iter().any(|r| r.file == "nvngx_dlssnr.dll") {
                        w::badge(ui, "DLSS 5", p.green);
                    } else if !self.runtimes.is_empty() {
                        w::badge(ui, "Installed", p.green);
                    }
                });
            });
        });
    }

    fn page_settings(&mut self, ui: &mut Ui, p: &Palette) {
        self.header(ui, p, "Settings", "Where things are and how they look");

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(680.0);

            ui.label(w::section("GAME", p));
            ui.add_space(6.0);
            w::grouped(ui, |ui| {
                let (root, skse) = match &self.game {
                    Some(g) => (Some(g.root.clone()), g.skse().is_some()),
                    None => (None, false),
                };
                w::row(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(w::title("Skyrim Special Edition"));
                        match &root {
                            Some(r) => ui.label(w::secondary(&r.display().to_string(), p)),
                            None => ui.label(RichText::new("Not found").size(12.5).color(p.red)),
                        };
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::pill(ui, None, "Change…", PillStyle::Secondary).clicked() {
                            self.pick_game(ui.ctx());
                        }
                        if let Some(r) = &root
                            && w::pill(ui, Some(icon::FOLDER), "Show", PillStyle::Ghost).clicked()
                        {
                            Self::open_folder(&r.join("Data"));
                        }
                    });
                });
                w::hairline(ui);
                w::row(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(w::title("Script Extender (SKSE)"));
                        ui.label(w::secondary(
                            if skse {
                                "Installed — Play launches through SKSE"
                            } else {
                                "Not installed — many mods need it. Play launches the game directly."
                            },
                            p,
                        ));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !skse && w::pill(ui, Some(icon::OPEN), "Get SKSE", PillStyle::Ghost).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(SKSE_URL));
                        }
                    });
                });
                w::hairline(ui);
                let logger = self.crash_logger_installed();
                let busy = self.fetching_files.contains(&crash::LOGGER_MOD_ID)
                    || self.downloads.iter().any(|d| d.mod_id == crash::LOGGER_MOD_ID);
                w::row(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(w::title("Crash reports"));
                        ui.label(w::secondary(
                            if logger {
                                "Crash Logger is installed — when Skyrim crashes, the reason shows up in the Library"
                            } else {
                                "Not set up — install Crash Logger so a crash leaves a report you can read here"
                            },
                            p,
                        ));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if busy {
                            w::spinner(ui, p.accent);
                        } else if !logger && skse && w::pill(ui, Some(icon::DOWNLOAD), "Install", PillStyle::Secondary).clicked() {
                            self.install_crash_logger(ui.ctx());
                        }
                        if let Some(dir) = crash::logs_dir()
                            && dir.is_dir()
                            && w::pill(ui, Some(icon::FOLDER), "Show", PillStyle::Ghost).on_hover_text("Open the folder with every report").clicked()
                        {
                            Self::open_folder(&dir);
                        }
                    });
                });
            });

            ui.add_space(24.0);
            ui.label(w::section("NEXUS MODS", p));
            ui.add_space(6.0);
            w::grouped(ui, |ui| {
                w::row(ui, |ui| {
                    match self.nexus_user.clone() {
                        Some(u) => {
                            ui.vertical(|ui| {
                                ui.label(w::title(&format!("Connected as {}", u.name)));
                                ui.label(w::secondary(
                                    if u.is_premium {
                                        "Premium — mods download straight into the app from Browse"
                                    } else {
                                        "Free account — use “Mod Manager Download” on the website and it lands here"
                                    },
                                    p,
                                ));
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if w::pill(ui, None, "Disconnect", PillStyle::Secondary).clicked() {
                                    self.disconnect();
                                }
                                if u.is_premium {
                                    w::badge(ui, "Premium", p.accent);
                                }
                            });
                        }
                        None => {
                            ui.vertical(|ui| {
                                ui.label(w::title("Connect your account"));
                                ui.label(w::secondary("Paste the Personal API Key from your Nexus Mods account page", p));
                                ui.add_space(8.0);
                                self.connect_controls(ui, p, 300.0);
                            });
                        }
                    }
                });
                w::hairline(ui);
                w::row(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(w::title("“Mod Manager Download” buttons"));
                        ui.label(w::secondary(
                            if self.protocol_ok {
                                "Ready — those buttons on nexusmods.com open in this app"
                            } else {
                                "Not set up — make this app handle Nexus download links"
                            },
                            p,
                        ));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.protocol_ok {
                            w::badge(ui, "Ready", p.green);
                        } else if w::pill(ui, None, "Set up", PillStyle::Secondary).clicked() {
                            match nexus::current_exe().map(|e| nexus::register_protocol(&e)) {
                                Some(Ok(())) => {
                                    self.protocol_ok = true;
                                    self.toast(ui.ctx(), ToastKind::Success, "Nexus download links now open here");
                                }
                                Some(Err(e)) => self.toast(ui.ctx(), ToastKind::Error, e.to_string()),
                                None => self.toast(ui.ctx(), ToastKind::Error, "Couldn't find this app's location"),
                            }
                        }
                    });
                });
            });

            ui.add_space(24.0);
            ui.label(w::section("GRAPHICS", p));
            ui.add_space(6.0);
            self.graphics_card(ui, p);

            ui.add_space(24.0);
            ui.label(w::section("APPEARANCE", p));
            ui.add_space(6.0);
            w::grouped(ui, |ui| {
                w::row(ui, |ui| {
                    ui.label(w::title("Theme"));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::segmented(ui, &mut self.settings.theme, &["Auto", "Light", "Dark"]) {
                            self.settings.apply_theme(ui.ctx());
                            self.settings.save(&self.paths);
                        }
                    });
                });
            });

            ui.add_space(24.0);
            ui.label(w::section("MOD FILES", p));
            ui.add_space(6.0);
            w::grouped(ui, |ui| {
                w::row(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(w::title("Storage"));
                        ui.label(w::secondary(&self.paths.mods_dir().display().to_string(), p));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::pill(ui, Some(icon::FOLDER), "Show", PillStyle::Ghost).clicked() {
                            Self::open_folder(&self.paths.mods_dir());
                        }
                    });
                });
                w::hairline(ui);
                w::row(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(w::title("Reapply all mods"));
                        ui.label(w::secondary("Puts every enabled mod back into the game from your library", p));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_enabled_ui(self.game.is_some() && self.busy.is_none(), |ui| {
                            if w::pill(ui, Some(icon::REFRESH), "Reapply", PillStyle::Secondary).clicked() {
                                self.needs_apply = true;
                            }
                        });
                    });
                });
                w::hairline(ui);
                w::row(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(w::title("Restore vanilla game"));
                        ui.label(w::secondary("Removes every mod file from the game. Your library is kept.", p));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_enabled_ui(self.game.is_some() && self.busy.is_none(), |ui| {
                            if w::pill(ui, None, "Restore…", PillStyle::Danger).clicked() {
                                self.confirm_restore = true;
                            }
                        });
                    });
                });
            });

            ui.add_space(24.0);
            ui.label(w::section("ABOUT", p));
            ui.add_space(6.0);
            w::grouped(ui, |ui| {
                w::row(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(w::title(&format!("Skyrim Mod Manager {}", env!("CARGO_PKG_VERSION"))));
                        ui.label(w::secondary("Drop mod archives, folders or single files (plugins, DLSS DLLs) anywhere in the window to install them.", p));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::pill(ui, Some(icon::OPEN), "Browse Nexus Mods", PillStyle::Ghost).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(NEXUS_URL));
                        }
                    });
                });
            });
            ui.add_space(24.0);
        });
    }

    // -----------------------------------------------------------------------
    // Shared pieces
    // -----------------------------------------------------------------------

    fn no_game_card(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(460.0);
            w::card(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.label(w::icon(icon::GAME, 40.0, p.accent));
                    ui.add_space(12.0);
                    ui.label(RichText::new("Where's Skyrim?").font(egui::FontId::new(20.0, theme::bold())));
                    ui.add_space(6.0);
                    ui.label(w::secondary("Couldn't find Skyrim Special Edition on this PC. Point me at SkyrimSE.exe and everything else is automatic.", p));
                    ui.add_space(16.0);
                    if w::pill(ui, Some(icon::FOLDER), "Locate SkyrimSE.exe", PillStyle::Primary).clicked() {
                        self.pick_game(ui.ctx());
                    }
                    ui.add_space(12.0);
                });
            });
        });
    }

    fn empty_library(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(520.0);
            Frame::new()
                .stroke(egui::Stroke::new(1.5, p.stroke))
                .corner_radius(egui::CornerRadius::same(16))
                .inner_margin(Margin::same(32))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(12.0);
                        ui.label(w::icon(icon::DOWNLOAD, 44.0, p.text_tertiary));
                        ui.add_space(14.0);
                        ui.label(RichText::new("Drop a mod here").font(egui::FontId::new(20.0, theme::bold())));
                        ui.add_space(6.0);
                        ui.label(w::secondary("Download a .zip or .7z from Nexus Mods and drag it into this window. It's installed and switched on automatically.", p));
                        ui.add_space(18.0);
                        ui.horizontal(|ui| {
                            let total = 118.0 + 8.0 + 170.0;
                            ui.add_space((ui.available_width() - total).max(0.0) / 2.0);
                            if w::pill(ui, Some(icon::ADD), "Add Mod", PillStyle::Primary).clicked() {
                                self.pick_mod_files();
                            }
                            if w::pill(ui, Some(icon::SEARCH), "Browse Nexus Mods", PillStyle::Ghost).clicked() {
                                self.page = Page::Browse;
                                self.search.clear();
                                if self.browse.query.is_none() {
                                    self.browse.typed.clear();
                                    self.run_search(ui.ctx(), 0);
                                }
                            }
                        });
                        ui.add_space(12.0);
                    });
                });
        });
    }

    /// "Skyrim crashed" when there's a fresh report — or, with nothing set up to write
    /// one, the offer to install Crash Logger so the next crash isn't a mystery.
    fn crash_notices(&mut self, ui: &mut Ui, p: &Palette) {
        if let Some(c) = &self.crash {
            let mut text = format!("Skyrim crashed {}.", crash::ago(c.report.time));
            match c.suspects.first() {
                Some((_, Some(m))) => text.push_str(&format!(" The crash points at {m}.")),
                Some((f, None)) => text.push_str(&format!(" The crash points at {f}.")),
                None => text.push_str(" No mod was named."),
            }
            self.notice(ui, p, p.red, &text, Some("Details"), |s| s.show_crash = true);
        } else if self.game.as_ref().is_some_and(|g| g.skse().is_some()) && !self.crash_logger_installed() {
            let ctx = ui.ctx().clone();
            self.notice(
                ui,
                p,
                p.accent,
                "If Skyrim crashes, nothing records why yet. Crash Logger writes a report you can read here.",
                Some("Install Crash Logger"),
                move |s| s.install_crash_logger(&ctx),
            );
        }
    }

    fn install_crash_logger(&mut self, ctx: &egui::Context) {
        if self.nexus.is_none() {
            self.toast(ctx, ToastKind::Error, "Connect your Nexus Mods account in Settings first");
            return;
        }
        self.install_from_nexus(ctx, crash::LOGGER_MOD_ID, crash::LOGGER_NAME, "", None, false);
    }

    /// Mods that can't work yet because something they depend on isn't installed. The two
    /// things the app can fix itself (SKSE, Address Library) get a button; the rest point
    /// to the orange text on their rows.
    fn needs_notices(&mut self, ui: &mut Ui, p: &Palette, missing: &HashMap<String, Missing>) {
        if missing.is_empty() {
            return;
        }
        let count = |f: fn(&Missing) -> bool| missing.values().filter(|m| f(m)).count();
        let mods = |n: usize| if n == 1 { "1 mod needs".to_string() } else { format!("{n} mods need") };
        let skse = count(|m| m.skse);
        let address = count(|m| m.address_library);
        if skse > 0 {
            let ctx = ui.ctx().clone();
            let text = format!("{} SKSE, which isn't installed. Download it and drop the archive in here.", mods(skse));
            self.notice(ui, p, p.orange, &text, Some("Get SKSE"), move |_| ctx.open_url(egui::OpenUrl::new_tab(SKSE_URL)));
        }
        if address > 0 {
            let ctx = ui.ctx().clone();
            let text = format!("{} Address Library, which isn't installed.", mods(address));
            self.notice(ui, p, p.orange, &text, Some("Install Address Library"), move |s| s.install_address_library(&ctx));
        }

        // Library mods that are switched off but needed — one click turns them back on.
        let mut off: Vec<String> = Vec::new();
        let mut not_installed: Vec<String> = Vec::new();
        for m in missing.values() {
            for o in &m.off {
                push_unique(&mut off, o.clone());
            }
            for m in &m.masters {
                push_unique(&mut not_installed, needs::display_name(m));
            }
        }
        let needing = |f: fn(&Missing) -> bool| -> String {
            let names: Vec<String> = self
                .library
                .mods
                .iter()
                .filter(|o| missing.get(&o.id).is_some_and(f))
                .map(|o| o.name.clone())
                .collect();
            if names.len() == 1 { names[0].clone() } else { format!("{} mods", names.len()) }
        };
        let (off_who, not_installed_who) = (needing(|m| !m.off.is_empty()), needing(|m| !m.masters.is_empty()));
        if !off.is_empty() {
            let (text, action) = if off.len() == 1 {
                (format!("{off_who} needs {}, which is turned off.", off[0]), "Turn it on")
            } else {
                (format!("{off_who} need mods that are turned off: {}.", join_names(&off)), "Turn them on")
            };
            self.notice(ui, p, p.orange, &text, Some(action), move |s| s.enable_mods(&off));
        }
        if !not_installed.is_empty() {
            let (text, action) = if not_installed.len() == 1 {
                (format!("{not_installed_who} needs {}, which isn't installed.", not_installed[0]), "Find it on Nexus")
            } else {
                (
                    format!(
                        "{not_installed_who} need mods that aren't installed: {}. Look for the orange line under each one.",
                        join_names(&not_installed)
                    ),
                    "Find on Nexus",
                )
            };
            let first = not_installed[0].clone();
            self.notice(ui, p, p.orange, &text, Some(action), move |s| s.find_on_nexus(first));
        }

        // Mods whose SKSE plugins were built for another version of the game.
        let outdated: Vec<(String, Option<u64>, bool)> = self
            .library
            .mods
            .iter()
            .filter_map(|o| {
                let x = missing.get(&o.id)?;
                let worst = x.outdated.iter().find(|o| o.certain).or(x.outdated.first())?;
                Some((o.name.clone(), o.nexus.as_ref().map(|n| n.mod_id), worst.certain))
            })
            .collect();
        if !outdated.is_empty() {
            let game = self.game_label();
            let names: Vec<String> = outdated.iter().map(|(n, _, _)| n.clone()).collect();
            let certain = outdated.iter().any(|(_, _, c)| *c);
            let outcome = if certain { "won't load" } else { "may not work" };
            let text = match names.len() {
                1 => format!("{} {outcome} with Skyrim {game} — it was made for an older version.", names[0]),
                n => format!("{n} mods {outcome} with Skyrim {game} — they were made for older versions: {}.", join_names(&names)),
            };
            let (name, mod_id, _) = outdated[0].clone();
            let ctx = ui.ctx().clone();
            self.notice(ui, p, p.orange, &text, Some("Look on Nexus"), move |s| match mod_id {
                Some(id) => ctx.open_url(egui::OpenUrl::new_tab(format!("{NEXUS_URL}{id}"))),
                None => s.find_on_nexus(name),
            });
        }
    }

    /// Switches the named Library mods on and applies the change.
    fn enable_mods(&mut self, names: &[String]) {
        let mut changed = false;
        for m in &mut self.library.mods {
            if !m.enabled && names.iter().any(|n| n == &m.name) {
                m.enabled = true;
                changed = true;
            }
        }
        if changed {
            let _ = self.library.save(&self.paths);
            self.needs_apply = true;
        }
    }

    fn install_address_library(&mut self, ctx: &egui::Context) {
        if self.nexus.is_none() {
            self.toast(ctx, ToastKind::Error, "Connect your Nexus Mods account in Settings first");
            return;
        }
        self.install_from_nexus(ctx, needs::ADDRESS_LIBRARY_MOD_ID, "Address Library for SKSE Plugins", "", None, false);
    }

    /// Jumps to Browse with a search for the mod that provides a missing plugin.
    fn find_on_nexus(&mut self, name: String) {
        self.search = name;
        self.page = Page::Browse;
    }

    /// What the crash report says, in plain words, with a way to act on it.
    fn crash_modal(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(c) = &self.crash else { return };
        let (report, suspects) = (c.report.clone(), c.suspects.clone());
        let mut close = false;
        let mut done = false;
        let modal = egui::Modal::new(egui::Id::new("crash"))
            .frame(w::card_frame(p).inner_margin(Margin::same(22)))
            .show(ctx, |ui| {
                ui.set_width(480.0);
                ui.label(RichText::new("Skyrim crashed").font(egui::FontId::new(17.0, theme::bold())));
                ui.add_space(4.0);
                let mut meta = crash::ago(report.time);
                if !report.error.is_empty() {
                    meta.push_str(" · ");
                    meta.push_str(&report.error.replace("EXCEPTION_", "").replace('_', " ").to_ascii_lowercase());
                }
                if !report.location.is_empty() {
                    meta.push_str(" in ");
                    meta.push_str(&report.location);
                }
                ui.label(w::secondary(&meta, p));
                ui.add_space(14.0);

                if suspects.is_empty() {
                    ui.label(RichText::new("The crash happened inside Skyrim itself and the report doesn't name a mod. That's usually a save that depends on a mod you removed, a plugin missing one of its masters, or a broken mesh or texture.").color(p.text));
                } else {
                    ui.label(w::section("FILES INVOLVED", p));
                    ui.add_space(6.0);
                    w::grouped(ui, |ui| {
                        let n = suspects.len();
                        for (i, (file, owner)) in suspects.iter().enumerate() {
                            w::row(ui, |ui| {
                                ui.vertical(|ui| {
                                    ui.add(egui::Label::new(w::title(file)).truncate());
                                    ui.label(w::secondary(
                                        &match owner {
                                            Some(m) => format!("From {m}"),
                                            None => "Not from a mod in your Library".to_string(),
                                        },
                                        p,
                                    ));
                                });
                            });
                            if i + 1 < n {
                                w::hairline(ui);
                            }
                        }
                    });
                    ui.add_space(10.0);
                    let hint = match &suspects[0].1 {
                        Some(m) => format!("Try turning off {m} in the Library and playing again. If the crash stops, that's the one — check its Nexus page for a fix or an update."),
                        None => "The first file is the most likely cause. It isn't from a mod installed here, so look for it in your game folder or in mods you added another way.".to_string(),
                    };
                    ui.label(RichText::new(hint).color(p.text));
                }
                ui.add_space(18.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::pill(ui, None, "Done", PillStyle::Primary).clicked() {
                        done = true;
                    }
                    if w::pill(ui, None, "Later", PillStyle::Secondary).clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if w::pill(ui, Some(icon::FOLDER), "Open report", PillStyle::Ghost).clicked() {
                            let _ = std::process::Command::new("notepad").arg(&report.path).spawn();
                        }
                        if w::pill(ui, None, "Copy", PillStyle::Ghost).on_hover_text("Copy the full report to paste when asking for help").clicked() {
                            if let Ok(text) = std::fs::read_to_string(&report.path) {
                                ui.ctx().copy_text(text);
                            }
                        }
                    });
                });
            });
        if done {
            self.dismiss_crash();
        } else if close || modal.should_close() {
            self.show_crash = false;
        }
    }

    /// What a mod needs, from its Nexus page, with a way to get each thing without leaving.
    fn requirements_sheet(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some((mod_id, name)) = self.req_sheet.clone() else { return };
        let reqs = self.requirements.get(&mod_id).cloned();
        let premium = self.is_premium();
        // Each row's state and action are worked out up front so the closure needn't borrow self.
        enum Fix {
            None,
            Installed,
            Skse,
            AddressLibrary,
            Nexus(u64, String),
            Site(String),
        }
        let rows: Vec<(String, String, Fix)> = reqs
            .as_ref()
            .and_then(|r| r.as_ref())
            .map(|r| {
                r.mods
                    .iter()
                    .map(|req| {
                        let fix = match (req.mod_id, self.requirement_met(req)) {
                            (_, Some(true)) => Fix::Installed,
                            (0, _) if req.url.is_empty() => Fix::None,
                            (0, _) => Fix::Site(req.url.clone()),
                            (nexus::SKSE_MOD_ID, _) => Fix::Skse,
                            (needs::ADDRESS_LIBRARY_MOD_ID, _) => Fix::AddressLibrary,
                            (id, _) => Fix::Nexus(id, req.name.clone()),
                        };
                        (req.name.clone(), req.notes.clone(), fix)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let dlc: Vec<String> = reqs.as_ref().and_then(|r| r.as_ref()).map(|r| r.dlc.clone()).unwrap_or_default();

        let mut close = false;
        let mut install_al = false;
        let mut install: Option<(u64, String)> = None;
        let modal = egui::Modal::new(egui::Id::new("requirements"))
            .frame(w::card_frame(p).inner_margin(Margin::same(22)))
            .show(ctx, |ui| {
                ui.set_width(480.0);
                ui.add(egui::Label::new(RichText::new(format!("{name} needs")).font(egui::FontId::new(17.0, theme::bold()))).truncate());
                ui.add_space(4.0);
                ui.label(w::secondary("From the mod's Nexus page. Everything here has to be installed and turned on.", p));
                ui.add_space(14.0);

                match &reqs {
                    None => {
                        ui.horizontal(|ui| {
                            w::spinner(ui, p.accent);
                            ui.add_space(6.0);
                            ui.label(RichText::new("Checking the Nexus page…").color(p.text));
                        });
                    }
                    Some(_) if rows.is_empty() && dlc.is_empty() => {
                        ui.label(RichText::new("Nothing — it works on its own.").color(p.text));
                    }
                    Some(_) => {
                        if !rows.is_empty() {
                            ui.label(w::section("MODS", p));
                            ui.add_space(6.0);
                            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                                w::grouped(ui, |ui| {
                                    let n = rows.len();
                                    for (i, (title, notes, fix)) in rows.iter().enumerate() {
                                        w::row(ui, |ui| {
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                match fix {
                                                    Fix::None => {}
                                                    Fix::Installed => w::badge(ui, "Installed", p.green),
                                                    Fix::Skse => {
                                                        if w::pill(ui, Some(icon::OPEN), "Get SKSE", PillStyle::Secondary)
                                                            .on_hover_text("SKSE lives on its own site, not Nexus")
                                                            .clicked()
                                                        {
                                                            ui.ctx().open_url(egui::OpenUrl::new_tab(SKSE_URL));
                                                        }
                                                    }
                                                    Fix::AddressLibrary => {
                                                        if w::pill(ui, Some(icon::DOWNLOAD), "Install", PillStyle::Primary).clicked() {
                                                            install_al = true;
                                                        }
                                                    }
                                                    Fix::Nexus(id, name) => {
                                                        if premium {
                                                            if w::pill(ui, Some(icon::DOWNLOAD), "Install", PillStyle::Primary).clicked() {
                                                                install = Some((*id, name.clone()));
                                                            }
                                                        } else if w::pill(ui, Some(icon::OPEN), "Get", PillStyle::Secondary)
                                                            .on_hover_text("Opens the mod on Nexus Mods — use “Mod Manager Download” there")
                                                            .clicked()
                                                        {
                                                            ui.ctx().open_url(egui::OpenUrl::new_tab(format!("{NEXUS_URL}{id}")));
                                                        }
                                                    }
                                                    Fix::Site(url) => {
                                                        if w::pill(ui, Some(icon::OPEN), "Open", PillStyle::Ghost)
                                                            .on_hover_text(url)
                                                            .clicked()
                                                        {
                                                            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                                                        }
                                                    }
                                                }
                                                ui.add_space(8.0);
                                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                                    ui.vertical(|ui| {
                                                        ui.add(egui::Label::new(w::title(title)).truncate());
                                                        if !notes.is_empty() {
                                                            ui.label(w::secondary(notes, p));
                                                        }
                                                    });
                                                });
                                            });
                                        });
                                        if i + 1 < n {
                                            w::hairline(ui);
                                        }
                                    }
                                });
                            });
                        }
                        if !dlc.is_empty() {
                            if !rows.is_empty() {
                                ui.add_space(14.0);
                            }
                            ui.label(w::section("CREATION CLUB CONTENT", p));
                            ui.add_space(6.0);
                            w::grouped(ui, |ui| {
                                w::row(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.add(egui::Label::new(w::title(&join_names(&dlc))).truncate());
                                        ui.label(w::secondary("Part of the Anniversary Upgrade, which you get from Steam", p));
                                    });
                                });
                            });
                        }
                    }
                }

                ui.add_space(18.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::pill(ui, None, "Done", PillStyle::Primary).clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if w::pill(ui, Some(icon::OPEN), "Open on Nexus Mods", PillStyle::Ghost).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(format!("{NEXUS_URL}{mod_id}")));
                        }
                    });
                });
            });
        if install_al {
            self.install_address_library(ctx);
        }
        if let Some((id, name)) = install {
            self.install_from_nexus(ctx, id, &name, "", None, false);
        }
        if close || modal.should_close() {
            self.req_sheet = None;
        }
    }

    fn notice(&mut self, ui: &mut Ui, p: &Palette, color: Color32, text: &str, action: Option<&str>, on_action: impl FnOnce(&mut Self)) {
        Frame::new()
            .fill(color.gamma_multiply(0.12))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                // The button goes down first so a long message truncates instead of running under it.
                let size = vec2(ui.available_width(), 28.0);
                ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
                    if let Some(a) = action
                        && w::pill(ui, None, a, PillStyle::Ghost).clicked()
                    {
                        on_action(self);
                    }
                    ui.add_space(8.0);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.label(w::icon(icon::WARNING, 14.0, color));
                        ui.add(egui::Label::new(RichText::new(text).color(p.text)).truncate());
                    });
                });
            });
        ui.add_space(12.0);
    }

    fn overlays(&mut self, ctx: &egui::Context, p: &Palette) {
        // Drag-and-drop highlight
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
            painter.rect_filled(screen, 0.0, p.accent.gamma_multiply(0.10));
            painter.rect_stroke(screen.shrink(12.0), 16.0, egui::Stroke::new(2.0, p.accent), egui::StrokeKind::Inside);
            painter.text(
                screen.center(),
                Align2::CENTER_CENTER,
                "Drop to install",
                egui::FontId::new(24.0, theme::bold()),
                p.accent,
            );
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for path in dropped {
            if path.is_dir() || crate::archive::is_supported(&path) || mods::is_loose_file(&path) {
                self.queue_install(path);
            } else {
                self.toast(
                    ctx,
                    ToastKind::Error,
                    "Drop a .zip or .7z archive, a folder, or a plugin/DLL the game loads",
                );
            }
        }

        // Toast
        if let Some(t) = &self.toast {
            let now = ctx.input(|i| i.time);
            if now > t.until {
                self.toast = None;
            } else {
                let (fg, glyph) = match t.kind {
                    ToastKind::Success => (p.green, icon::CHECK),
                    ToastKind::Error => (p.red, icon::WARNING),
                };
                let text = t.text.clone();
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(ctx, |ui| {
                        Frame::new()
                            .fill(p.card)
                            .stroke(egui::Stroke::new(1.0, p.stroke))
                            .corner_radius(egui::CornerRadius::same(20))
                            .shadow(ui.style().visuals.popup_shadow)
                            .inner_margin(Margin::symmetric(16, 10))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(w::icon(glyph, 14.0, fg));
                                    ui.label(RichText::new(text).size(13.5));
                                });
                            });
                    });
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }

        // Confirmations
        if let Some(id) = self.confirm_remove.clone() {
            let name = self
                .library
                .mods
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.name.clone())
                .unwrap_or_default();
            let mut close = false;
            let modal = egui::Modal::new(egui::Id::new("confirm-remove"))
                .frame(w::card_frame(p).inner_margin(Margin::same(22)))
                .show(ctx, |ui| {
                    ui.set_width(340.0);
                    ui.label(RichText::new(format!("Remove \"{name}\"?")).font(egui::FontId::new(17.0, theme::bold())));
                    ui.add_space(6.0);
                    ui.label(w::secondary("Its files are taken out of the game and deleted from your library. You can always install it again.", p));
                    ui.add_space(18.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::pill(ui, None, "Remove", PillStyle::Danger).clicked() {
                            if let Some(i) = self.library.mods.iter().position(|m| m.id == id) {
                                self.library.mods.remove(i);
                                let _ = self.library.save(&self.paths);
                                mods::remove_mod_files(&self.paths, &id);
                                self.needs.remove(&id);
                                self.needs_apply = true;
                            }
                            close = true;
                        }
                        if w::pill(ui, None, "Cancel", PillStyle::Secondary).clicked() {
                            close = true;
                        }
                    });
                });
            if close || modal.should_close() {
                self.confirm_remove = None;
            }
        }

        if let Some(pick) = &self.file_pick {
            let mut close = false;
            let mut chosen: Option<FileInfo> = None;
            let (mod_id, name, version, replaces) = (pick.mod_id, pick.name.clone(), pick.version.clone(), pick.replaces.clone());
            let files = pick.files.clone();
            let multi_part = files.iter().filter(|f| f.is_main()).count() > 1;
            // Files already in the library, being downloaded, or waiting to install.
            let have: HashSet<u64> = self
                .library
                .mods
                .iter()
                .filter_map(|m| m.nexus.as_ref())
                .filter(|n| n.mod_id == mod_id)
                .map(|n| n.file_id)
                .collect();
            let active: HashSet<u64> = self
                .downloads
                .iter()
                .filter(|d| d.mod_id == mod_id)
                .map(|d| d.file_id)
                .chain(
                    self.pending_installs
                        .iter()
                        .chain(self.installing.iter())
                        .filter_map(|p| p.nexus.as_ref())
                        .filter(|n| n.mod_id == mod_id)
                        .map(|n| n.file_id),
                )
                .collect();
            let modal = egui::Modal::new(egui::Id::new("file-pick"))
                .frame(w::card_frame(p).inner_margin(Margin::same(22)))
                .show(ctx, |ui| {
                    ui.set_width(460.0);
                    ui.label(RichText::new(format!("Which part of \"{name}\"?")).font(egui::FontId::new(17.0, theme::bold())));
                    ui.add_space(4.0);
                    ui.label(w::secondary(
                        if replaces.is_some() {
                            "Pick the file that matches what you installed before."
                        } else {
                            "This mod comes in more than one file. Main files are what most people want."
                        },
                        p,
                    ));
                    ui.add_space(14.0);
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        w::grouped(ui, |ui| {
                            let mut ordered: Vec<&FileInfo> = files.iter().collect();
                            ordered.sort_by_key(|f| match f.category() {
                                "MAIN" => 0,
                                "UPDATE" => 1,
                                "OPTIONAL" => 2,
                                _ => 3,
                            });
                            let n = ordered.len();
                            for (i, f) in ordered.into_iter().enumerate() {
                                w::row(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.set_max_width(ui.available_width() - 120.0);
                                        ui.label(w::title(f.name.trim().trim_start_matches(['-', '_']).trim()));
                                        let mut meta = Vec::new();
                                        meta.push(match f.category() {
                                            "MAIN" => "Main file".to_string(),
                                            "UPDATE" => "Update".to_string(),
                                            "OPTIONAL" => "Optional".to_string(),
                                            _ => "Miscellaneous".to_string(),
                                        });
                                        if !f.version.is_empty() {
                                            meta.push(format!("v{}", f.version));
                                        }
                                        meta.push(mods::format_size(f.size_kb * 1024));
                                        ui.label(w::secondary(&meta.join(" · "), p));
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if active.contains(&f.file_id) {
                                            w::spinner(ui, p.accent);
                                        } else if have.contains(&f.file_id) && replaces.is_none() {
                                            w::badge(ui, "Installed", p.green);
                                        } else {
                                            let style = if f.is_main() { PillStyle::Primary } else { PillStyle::Secondary };
                                            if w::pill(ui, Some(icon::DOWNLOAD), "Install", style).clicked() {
                                                chosen = Some(f.clone());
                                            }
                                        }
                                    });
                                });
                                if i + 1 < n {
                                    w::hairline(ui);
                                }
                            }
                        });
                    });
                    ui.add_space(14.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let label = if multi_part && !active.is_empty() { "Done" } else { "Cancel" };
                        if w::pill(ui, None, label, PillStyle::Secondary).clicked() {
                            close = true;
                        }
                        if multi_part {
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.label(w::secondary("Big mods ship in parts — install every main part.", p));
                            });
                        }
                    });
                });
            if let Some(f) = chosen {
                // Updates replace one file; multi-part mods keep the picker open for the rest.
                if !multi_part || replaces.is_some() {
                    self.file_pick = None;
                }
                self.start_download(ctx, mod_id, &name, &version, Some(f), None, replaces, multi_part);
            } else if close || modal.should_close() {
                self.file_pick = None;
            }
        }

        if self.confirm_restore {
            let mut close = false;
            let modal = egui::Modal::new(egui::Id::new("confirm-restore"))
                .frame(w::card_frame(p).inner_margin(Margin::same(22)))
                .show(ctx, |ui| {
                    ui.set_width(340.0);
                    ui.label(RichText::new("Restore the vanilla game?").font(egui::FontId::new(17.0, theme::bold())));
                    ui.add_space(6.0);
                    ui.label(w::secondary("Every mod file is removed from Skyrim's folders and any original files are put back. Your library stays as it is — use Reapply to bring mods back.", p));
                    ui.add_space(18.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::pill(ui, None, "Restore", PillStyle::Danger).clicked() {
                            if let Some(game) = self.game.clone() {
                                self.busy = Some(Busy::Restoring);
                                let tx = self.tx.clone();
                                let paths = self.paths.clone();
                                let ctx2 = ctx.clone();
                                std::thread::spawn(move || {
                                    let r = mods::undeploy(&paths, &game).map_err(|e| e.to_string());
                                    let _ = tx.send(Msg::Restored(r));
                                    ctx2.request_repaint();
                                });
                            }
                            close = true;
                        }
                        if w::pill(ui, None, "Cancel", PillStyle::Secondary).clicked() {
                            close = true;
                        }
                    });
                });
            if close || modal.should_close() {
                self.confirm_restore = false;
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.pump(&ctx);
        let p = theme::palette(&ctx);

        egui::Panel::left("sidebar")
            .exact_size(240.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(p.sidebar)
                    .inner_margin(Margin { left: 12, right: 12, top: 14, bottom: 16 }),
            )
            .show(ui, |ui| self.sidebar(ui, &p));

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(p.window)
                    .inner_margin(Margin { left: 32, right: 32, top: 26, bottom: 20 }),
            )
            .show(ui, |ui| match self.page {
                Page::Library => self.page_library(ui, &p),
                Page::Browse => self.page_browse(ui, &p),
                Page::Plugins => self.page_plugins(ui, &p),
                Page::Settings => self.page_settings(ui, &p),
            });

        self.overlays(&ctx, &p);
        if self.req_sheet.is_some() {
            self.requirements_sheet(&ctx, &p);
        }
        if self.show_crash {
            self.crash_modal(&ctx, &p);
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, category: &str) -> FileInfo {
        FileInfo {
            file_id: 1,
            name: name.into(),
            version: "1.0".into(),
            category_name: Some(category.into()),
            file_name: String::new(),
            size_kb: 0,
        }
    }

    #[test]
    fn replacements() {
        let f = |id: u64, category: &str| FileInfo { file_id: id, ..file("f", category) };
        let files = vec![f(1, "OLD_VERSION"), f(2, "MAIN"), f(3, "OPTIONAL")];
        let existing = vec![("old".to_string(), 1), ("patch".to_string(), 3)];
        // The same file again: reinstall over its entry.
        assert_eq!(replacement_for(&existing, &files, 3, false).as_deref(), Some("patch"));
        // The new main file takes the place of the old main file, not the patch.
        assert_eq!(replacement_for(&existing, &files, 2, false).as_deref(), Some("old"));
        // …unless it's one part of many.
        assert_eq!(replacement_for(&existing, &files, 2, true), None);
        // A file Nexus no longer lists counts as an old main file.
        assert_eq!(replacement_for(&[("gone".into(), 99)], &files, 2, false).as_deref(), Some("gone"));
        // Optional files sit alongside what's there.
        assert_eq!(replacement_for(&[("main".into(), 2)], &files, 3, false), None);
        // Nothing from this mod yet.
        assert_eq!(replacement_for(&[], &files, 2, false), None);
    }

    #[test]
    fn dragging() {
        // Three rows 40px tall, showing full-list items 2, 5 and 7 (a filtered view).
        let rows: Vec<(usize, egui::Rect)> = [2, 5, 7]
            .iter()
            .enumerate()
            .map(|(k, &i)| (i, egui::Rect::from_min_size(egui::pos2(0.0, k as f32 * 40.0), vec2(100.0, 40.0))))
            .collect();
        assert_eq!(drop_slot(&rows, -10.0), 0);
        assert_eq!(drop_slot(&rows, 10.0), 0); // above the first row's middle
        assert_eq!(drop_slot(&rows, 30.0), 1); // below it
        assert_eq!(drop_slot(&rows, 90.0), 2);
        assert_eq!(drop_slot(&rows, 200.0), 3); // past the last row
        assert_eq!(drop_slot(&[], 5.0), 0);

        let mut v = vec!['a', 'b', 'c', 'd'];
        move_item(&mut v, 0, 3); // a before d
        assert_eq!(v, ['b', 'c', 'a', 'd']);
        move_item(&mut v, 2, 0); // a to the front
        assert_eq!(v, ['a', 'b', 'c', 'd']);
        move_item(&mut v, 1, 4); // b to the end
        assert_eq!(v, ['a', 'c', 'd', 'b']);
        move_item(&mut v, 1, 1); // no-op
        move_item(&mut v, 1, 2); // also a no-op
        assert_eq!(v, ['a', 'c', 'd', 'b']);
    }

    #[test]
    fn install_titles() {
        let arch = file("-Skyrim 202X 10.5.2 - Architecture PART 1", "MAIN");
        // A single main file keeps the mod's own name.
        assert_eq!(install_title("Skyrim 202X by Pfuscher", Some(&arch), false), "Skyrim 202X by Pfuscher");
        // Parts are named by their file when the file already says which mod it is…
        assert_eq!(install_title("Skyrim 202X by Pfuscher", Some(&arch), true), "Skyrim 202X 10.5.2 - Architecture PART 1");
        // …and prefixed with the mod otherwise.
        let opt = file("Whiterun Street - Alternative 1", "OPTIONAL");
        assert_eq!(install_title("Skyrim 202X by Pfuscher", Some(&opt), false), "Skyrim 202X by Pfuscher · Whiterun Street - Alternative 1");
        assert_eq!(install_title("Some Mod", None, true), "Some Mod");
    }
}
