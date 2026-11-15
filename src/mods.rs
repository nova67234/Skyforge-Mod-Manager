use crate::archive;
use crate::game::{self, Game, Plugin};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const DATA_FOLDERS: &[&str] = &[
    "meshes", "textures", "scripts", "interface", "sound", "music", "skse", "seq", "strings",
    "video", "shadersfx", "grass", "trees", "lodsettings", "facegendata", "dialogueviews",
    "mcm", "nemesis_engine", "calientetools", "platform", "dyndolod", "skyproc patchers",
];

/// Inside a stored mod, files that belong next to SkyrimSE.exe rather than in Data
/// (DLSS/upscaler DLLs, ENB, SKSE loader) live under this folder.
pub const ROOT_DIR: &str = "_root";

/// Files and folders that mods drop into the game's root folder. Kept to a known list so a
/// stray readme never lands next to the game executable.
fn is_root_item(name: &str, is_dir: bool) -> bool {
    const DIRS: &[&str] = &["enbseries", "reshade-shaders"];
    const FILES: &[&str] = &[
        // DirectX / injector hooks used by ENB, ReShade, OptiScaler and the upscaler mods
        "dxgi.dll", "d3d11.dll", "d3d12.dll", "d3d9.dll", "dinput8.dll", "winhttp.dll",
        "version.dll", "d3dcompiler_46e.dll", "d3dcompiler_47.dll",
        // SSE Engine Fixes part 2 (skse64 preloader + TBB)
        "d3dx9_42.dll", "tbb.dll", "tbbmalloc.dll",
        // NVIDIA DLSS runtimes and Streamline, plus the AMD / Intel equivalents
        "nvngx.dll", "_nvngx.dll", "nvapi64.dll", "nvapi64-proxy.dll", "fakenvapi.ini",
        "libxess.dll", "libxess_dx11.dll", "libxess_fg.dll",
        "dlssg_to_fsr3_amd_is_better.dll", "dlssg_to_fsr3.ini",
        "optiscaler.dll", "optiscaler.ini",
        // ENB and ReShade
        "enbseries.ini", "enblocal.ini", "enbhost.exe", "reshade.ini", "reshade64.dll",
    ];
    const PREFIXES: &[(&str, &[&str])] = &[
        ("nvngx_", &[".dll"]),
        ("sl.", &[".dll"]),
        ("amd_fidelityfx_", &[".dll"]),
        ("skse64_", &[".dll", ".exe"]),
    ];
    let name = name.to_ascii_lowercase();
    if is_dir {
        return DIRS.contains(&name.as_str());
    }
    FILES.contains(&name.as_str())
        || PREFIXES.iter().any(|(pre, exts)| {
            name.starts_with(pre) && exts.iter().any(|e| name.ends_with(e))
        })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mod {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub installed_at: u64,
    pub file_count: usize,
    pub size_bytes: u64,
    pub plugins: Vec<String>,
    #[serde(default)]
    pub source: String,
    /// Files this mod places next to SkyrimSE.exe (see `ROOT_DIR`).
    #[serde(default)]
    pub root_files: Vec<String>,
    /// Where on Nexus Mods this came from, when it was installed through the app.
    #[serde(default)]
    pub nexus: Option<NexusRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NexusRef {
    pub mod_id: u64,
    pub file_id: u64,
    /// The mod's version on Nexus when this file was installed; used to spot updates.
    pub version: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Library {
    /// Install order. Later mods override earlier ones when files conflict.
    pub mods: Vec<Mod>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DeployedFile {
    rel: String,
    mod_id: String,
    backed_up: bool,
    /// Placed in the game root instead of Data.
    #[serde(default)]
    root: bool,
}

impl DeployedFile {
    fn target(&self, game: &Game) -> PathBuf {
        if self.root { game.root.join(&self.rel) } else { game.data_dir().join(&self.rel) }
    }

    fn base(&self, game: &Game) -> PathBuf {
        if self.root { game.root.clone() } else { game.data_dir() }
    }

    fn backup(&self, paths: &Paths) -> PathBuf {
        if self.root {
            paths.backup_dir().join(ROOT_DIR).join(&self.rel)
        } else {
            paths.backup_dir().join(&self.rel)
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct DeployState {
    files: Vec<DeployedFile>,
}

#[derive(Clone, Debug, Default)]
pub struct DeployReport {
    pub files_linked: usize,
    pub conflicts: usize,
    pub copied_instead_of_linked: bool,
}

/// Where the manager keeps its own data.
#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn new() -> Result<Self> {
        let dirs = directories::ProjectDirs::from("", "", "Skyrim Mod Manager")
            .context("Couldn't find a place to store app data")?;
        let root = dirs.data_local_dir().to_path_buf();
        for sub in ["mods", "staging", "backup", "downloads", "thumbs"] {
            fs::create_dir_all(root.join(sub))?;
        }
        let paths = Self { root };
        paths.sweep();
        Ok(paths)
    }

    /// Nothing is in flight at startup, so anything left in staging or any
    /// half-finished download is debris from a crash or a killed process.
    fn sweep(&self) {
        let Ok(entries) = fs::read_dir(self.staging_dir()) else { return };
        for e in entries.flatten() {
            let _ = if e.path().is_dir() { fs::remove_dir_all(e.path()) } else { fs::remove_file(e.path()) };
        }
        let Ok(entries) = fs::read_dir(self.downloads_dir()) else { return };
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|x| x == "part") {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    pub fn mods_dir(&self) -> PathBuf {
        self.root.join("mods")
    }
    pub fn mod_dir(&self, id: &str) -> PathBuf {
        self.mods_dir().join(id)
    }
    fn staging_dir(&self) -> PathBuf {
        self.root.join("staging")
    }
    /// Archives fetched from Nexus Mods wait here until they're installed.
    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }
    /// Cached Nexus preview for a mod, so the Library has pictures offline too.
    pub fn thumb_file(&self, mod_id: u64) -> PathBuf {
        self.root.join("thumbs").join(format!("{mod_id}.png"))
    }
    /// Appends a line to `log.txt` so errors survive after their toast fades.
    pub fn log(&self, line: &str) {
        use std::io::Write;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.root.join("log.txt")) {
            let _ = writeln!(f, "{stamp} {line}");
        }
    }
    fn backup_dir(&self) -> PathBuf {
        self.root.join("backup")
    }
    fn library_file(&self) -> PathBuf {
        self.root.join("library.json")
    }
    fn deploy_file(&self) -> PathBuf {
        self.root.join("deployed.json")
    }
    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }
}

impl Library {
    pub fn load(paths: &Paths) -> Self {
        fs::read_to_string(paths.library_file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        fs::write(paths.library_file(), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// "SkyUI_5_2_SE-12604-5-2SE-1234567.7z" -> "SkyUI 5 2 SE"
pub fn pretty_name(file_stem: &str) -> String {
    let mut s = file_stem.to_string();
    // Nexus appends "-<modid>-<version>-<timestamp>"; cut at the first "-<digits>" group.
    // Dashes inside brackets are part of the name ("(2026-09-05)"), not a Nexus suffix.
    let bytes = s.as_bytes();
    let mut cut = None;
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = (depth - 1).max(0),
            b'-' if depth == 0 && i > 0 && bytes.get(i + 1).is_some_and(|c| c.is_ascii_digit()) => {
                cut = Some(i);
                break;
            }
            _ => {}
        }
    }
    if let Some(i) = cut {
        s.truncate(i);
    }
    let mut words: Vec<&str> = s.split(['_', ' ']).filter(|w| !w.is_empty()).collect();
    // Drop trailing version-ish words: "v1.2", "1.5.97", "SE" stays.
    while words.len() > 1
        && words.last().is_some_and(|w| {
            let w = w.trim_start_matches(['v', 'V']);
            !w.is_empty() && w.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
    {
        words.pop();
    }
    let s = words.join(" ").trim().to_string();
    if s.is_empty() { file_stem.to_string() } else { s }
}

fn dir_stats(dir: &Path) -> (usize, u64, Vec<String>) {
    let mut count = 0;
    let mut size = 0;
    let mut plugins = Vec::new();
    for entry in WalkDir::new(dir).into_iter().flatten() {
        if entry.file_type().is_file() {
            count += 1;
            size += entry.metadata().map(|m| m.len()).unwrap_or(0);
            if entry.depth() == 1 {
                let name = entry.file_name().to_string_lossy().to_string();
                if game::is_plugin_file(&name) {
                    plugins.push(name);
                }
            }
        }
    }
    (count, size, plugins)
}

fn looks_like_data_dir(dir: &Path) -> bool {
    let Ok(rd) = fs::read_dir(dir) else { return false };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let is_dir = entry.path().is_dir();
        if is_dir && DATA_FOLDERS.contains(&name.as_str()) {
            return true;
        }
        if !is_dir && (game::is_plugin_file(&name) || name.ends_with(".bsa")) {
            return true;
        }
    }
    false
}

/// Finds the folder inside an extracted archive that should map onto Skyrim/Data.
fn find_data_root(extracted: &Path) -> PathBuf {
    let mut best: Option<(usize, PathBuf)> = None;
    for entry in WalkDir::new(extracted).max_depth(4).into_iter().flatten() {
        if !entry.file_type().is_dir() {
            continue;
        }
        let p = entry.path();
        let depth = entry.depth();
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let is_data = name == "data" && depth > 0;
        if (is_data || looks_like_data_dir(p)) && best.as_ref().is_none_or(|(d, _)| depth < *d) {
            best = Some((depth, p.to_path_buf()));
        }
    }
    best.map(|(_, p)| p).unwrap_or_else(|| {
        // Single wrapper folder with nothing else? Unwrap it.
        let mut cur = extracted.to_path_buf();
        loop {
            let entries: Vec<_> = fs::read_dir(&cur).map(|r| r.flatten().collect()).unwrap_or_default();
            if entries.len() == 1 && entries[0].path().is_dir() {
                cur = entries[0].path();
            } else {
                break cur;
            }
        }
    })
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    for entry in WalkDir::new(src).into_iter().flatten() {
        let rel = entry.path().strip_prefix(src)?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else {
            if let Some(p) = target.parent() {
                fs::create_dir_all(p)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn move_dir(src: &Path, dst: &Path) -> Result<()> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    copy_dir(src, dst)?;
    let _ = fs::remove_dir_all(src);
    Ok(())
}

/// Installs a mod from a .zip/.7z archive or an already-extracted folder.
/// Does not deploy; call `deploy` afterwards.
pub fn install(paths: &Paths, source: &Path) -> Result<Mod> {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Mod".into());
    let name = pretty_name(&stem);
    let id = format!("{}-{}", sanitize(&name), now());
    let staging = paths.staging_dir().join(&id);
    let _ = fs::remove_dir_all(&staging);

    if source.is_dir() {
        copy_dir(source, &staging)?;
    } else if archive::is_supported(source) {
        archive::extract(source, &staging)?;
    } else {
        bail!("That doesn't look like a mod archive or folder.");
    }

    let data_root = find_data_root(&staging);
    let dest = paths.mod_dir(&id);
    let _ = fs::remove_dir_all(&dest);

    // Game-root files (DLSS DLLs, ENB, SKSE) sit beside the Data folder in the archive, or
    // mixed in with the game folders when there is no Data wrapper.
    let is_data_folder = data_root
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("data"));
    let beside_data = data_root.parent().map(Path::to_path_buf);

    move_dir(&data_root, &dest)?;
    let search = match (is_data_folder, beside_data) {
        (true, Some(parent)) => parent,
        _ => dest.clone(),
    };
    let root_files = collect_root_items(&search, &dest.join(ROOT_DIR))?;
    let _ = fs::remove_dir_all(&staging);

    let (file_count, size_bytes, plugins) = dir_stats(&dest);
    if file_count == 0 {
        let _ = fs::remove_dir_all(&dest);
        bail!("The archive was empty, or didn't contain any game files.");
    }

    Ok(Mod {
        id,
        name,
        enabled: true,
        installed_at: now(),
        file_count,
        size_bytes,
        plugins,
        source: source.display().to_string(),
        root_files,
        nexus: None,
    })
}

/// A single file that can be installed on its own: a plugin, an archive of assets, or one
/// of the DLLs/INIs the game or an upscaler loads directly.
pub fn is_loose_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    game::is_plugin_file(&name) || name.ends_with(".bsa") || is_root_item(&name, false)
}

/// Installs one loose file as its own mod. `into` is a Data-relative folder to place it in
/// (used for DLSS runtimes, which belong wherever the installed upscaler keeps them);
/// otherwise plugins and archives go to Data and known game-root files next to the exe.
/// Does not deploy; call `deploy` afterwards.
pub fn install_file(paths: &Paths, file: &Path, into: Option<&Path>) -> Result<Mod> {
    let file_name = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .context("That file has no name")?;
    let name = crate::gpu::runtime_label(&file_name)
        .map(|label| match crate::gpu::file_version(file) {
            Some(v) => format!("{label} {v}"),
            None => label.to_string(),
        })
        .unwrap_or_else(|| file_name.clone());
    let id = format!("{}-{}", sanitize(&name), now());
    let dest = paths.mod_dir(&id);
    let _ = fs::remove_dir_all(&dest);

    let mut root_files = Vec::new();
    let target = match into {
        Some(dir) => dest.join(dir).join(&file_name),
        None if is_root_item(&file_name, false) => {
            root_files.push(file_name.clone());
            dest.join(ROOT_DIR).join(&file_name)
        }
        None => dest.join(&file_name),
    };
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(file, &target).with_context(|| format!("Couldn't copy {file_name}"))?;

    let (file_count, size_bytes, plugins) = dir_stats(&dest);
    Ok(Mod {
        id,
        name,
        enabled: true,
        installed_at: now(),
        file_count,
        size_bytes,
        plugins,
        source: file.display().to_string(),
        root_files,
        nexus: None,
    })
}

/// Moves recognised game-root items found directly in `search` into `root_dest`, returning
/// their names. Anything unrecognised (readmes, source folders) is left alone.
fn collect_root_items(search: &Path, root_dest: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(search)?.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_root_item(&name, path.is_dir()) {
            continue;
        }
        fs::create_dir_all(root_dest)?;
        move_dir(&path, &root_dest.join(&name))?;
        names.push(name);
    }
    names.sort_unstable_by_key(|n| n.to_ascii_lowercase());
    Ok(names)
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn remove_mod_files(paths: &Paths, id: &str) {
    let _ = fs::remove_dir_all(paths.mod_dir(id));
}

fn load_deploy_state(paths: &Paths) -> DeployState {
    fs::read_to_string(paths.deploy_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn remove_empty_parents(file: &Path, stop: &Path) {
    let mut cur = file.parent();
    while let Some(dir) = cur {
        if dir == stop || fs::remove_dir(dir).is_err() {
            break;
        }
        cur = dir.parent();
    }
}

/// Removes everything we previously placed in the game folder and restores any backups.
pub fn undeploy(paths: &Paths, game: &Game) -> Result<()> {
    let state = load_deploy_state(paths);
    for f in &state.files {
        let target = f.target(game);
        let _ = fs::remove_file(&target);
        if f.backed_up {
            let backup = f.backup(paths);
            if backup.exists() {
                if let Some(p) = target.parent() {
                    fs::create_dir_all(p)?;
                }
                let _ = fs::rename(&backup, &target);
                remove_empty_parents(&backup, &paths.backup_dir());
            }
        } else {
            remove_empty_parents(&target, &f.base(game));
        }
    }
    fs::write(paths.deploy_file(), serde_json::to_string(&DeployState::default())?)?;
    Ok(())
}

/// Syncs Data with the enabled mods: hardlinks each file in (copying if linking fails),
/// backing up any vanilla file we overwrite. Also refreshes the plugin list.
pub fn deploy(paths: &Paths, game: &Game, library: &Library) -> Result<(DeployReport, Vec<Plugin>)> {
    undeploy(paths, game)?;
    let mut report = DeployReport::default();
    let mut placed: HashMap<String, DeployedFile> = HashMap::new();

    for m in library.mods.iter().filter(|m| m.enabled) {
        let src_root = paths.mod_dir(&m.id);
        for entry in WalkDir::new(&src_root).into_iter().flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry.path().strip_prefix(&src_root)?;
            // Files under `_root` go next to the game executable instead of into Data.
            let (root, rel) = match rel.strip_prefix(ROOT_DIR) {
                Ok(inner) => (true, inner),
                Err(_) => (false, rel),
            };
            let rel_str = rel.to_string_lossy().replace('/', "\\");
            let key = format!("{}{}", if root { "\\" } else { "" }, rel_str.to_ascii_lowercase());
            let mut file = DeployedFile { rel: rel_str, mod_id: m.id.clone(), backed_up: false, root };
            let target = file.target(game);
            if let Some(p) = target.parent() {
                fs::create_dir_all(p)?;
            }

            if let Some(prev) = placed.get(&key) {
                report.conflicts += 1;
                file.backed_up = prev.backed_up;
                let _ = fs::remove_file(&target);
            } else if target.exists() {
                let backup = file.backup(paths);
                if let Some(p) = backup.parent() {
                    fs::create_dir_all(p)?;
                }
                fs::rename(&target, &backup)
                    .with_context(|| format!("Couldn't back up {}", target.display()))?;
                file.backed_up = true;
            }

            if fs::hard_link(entry.path(), &target).is_err() {
                fs::copy(entry.path(), &target)
                    .with_context(|| format!("Couldn't copy {}", rel.display()))?;
                report.copied_instead_of_linked = true;
            }
            report.files_linked += 1;
            placed.insert(key, file);
        }
    }

    let state = DeployState { files: placed.into_values().collect() };
    fs::write(paths.deploy_file(), serde_json::to_string(&state)?)?;

    let plugins = game::scan_plugins(game);
    game::write_plugins_txt(&plugins)?;
    Ok((report, plugins))
}

pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.1} GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.0} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::pretty_name;

    #[test]
    fn pretty_names() {
        assert_eq!(pretty_name("SkyUI_5_2_SE-12604-5-2SE-1234567"), "SkyUI 5 2 SE");
        assert_eq!(pretty_name("Address Library All in One (1.7.104.0)"), "Address Library All in One (1.7.104.0)");
        assert_eq!(
            pretty_name("Community Shaders 1.9.0 dev (2026-09-05, supports 1.7.104) AIO"),
            "Community Shaders 1.9.0 dev (2026-09-05, supports 1.7.104) AIO"
        );
        assert_eq!(pretty_name("SSE Engine Fixes 7.0.21 (Part 1) for 1.7.104 - GitHub"), "SSE Engine Fixes 7.0.21 (Part 1) for 1.7.104 - GitHub");
        assert_eq!(pretty_name("Some Mod v1.2"), "Some Mod");
    }
}
