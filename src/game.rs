use anyhow::{Context, Result, anyhow, bail};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const EXE_NAME: &str = "SkyrimSE.exe";
const SKSE_LOADER: &str = "skse64_loader.exe";
/// Skyrim Special Edition's id on Steam.
#[cfg(not(windows))]
const STEAM_APP_ID: &str = "489830";

/// Plugins that ship with the game (or Creation Club) and are always loaded.
const CORE_PLUGINS: &[&str] = &[
    "Skyrim.esm",
    "Update.esm",
    "Dawnguard.esm",
    "HearthFires.esm",
    "Dragonborn.esm",
    "_ResourcePack.esl",
];

#[derive(Clone, Debug)]
pub struct Game {
    pub root: PathBuf,
}

impl Game {
    pub fn from_root(root: impl Into<PathBuf>) -> Option<Self> {
        let root = root.into();
        root.join(EXE_NAME).is_file().then_some(Self { root })
    }

    /// Accepts either the game folder or the path to SkyrimSE.exe.
    pub fn from_user_path(path: &Path) -> Option<Self> {
        if path.is_file() {
            path.parent().and_then(Self::from_root)
        } else {
            Self::from_root(path)
        }
    }

    pub fn detect() -> Option<Self> {
        registry_paths()
            .into_iter()
            .chain(steam_library_paths())
            .chain(common_paths())
            .find_map(Self::from_root)
    }

    pub fn data_dir(&self) -> PathBuf {
        self.root.join("Data")
    }

    pub fn skse(&self) -> Option<PathBuf> {
        let p = self.root.join(SKSE_LOADER);
        p.is_file().then_some(p)
    }

    #[cfg(windows)]
    pub fn launch(&self) -> Result<()> {
        let exe = self.skse().unwrap_or_else(|| self.root.join(EXE_NAME));
        Command::new(&exe)
            .current_dir(&self.root)
            .spawn()
            .with_context(|| format!("Couldn't start {}", exe.display()))?;
        Ok(())
    }

    /// Skyrim is a Windows game, so outside Windows it can't be started directly: Steam
    /// has to run it through Proton (or whichever compatibility layer it's set up with).
    #[cfg(not(windows))]
    pub fn launch(&self) -> Result<()> {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        Command::new(opener)
            .arg(format!("steam://rungameid/{STEAM_APP_ID}"))
            .spawn()
            .context("Couldn't ask Steam to start Skyrim")?;
        Ok(())
    }

    /// Directory the game reads Plugins.txt from.
    #[cfg(windows)]
    pub fn local_appdata_dir() -> Result<PathBuf> {
        let base = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| anyhow!("LOCALAPPDATA is not set"))?;
        Ok(PathBuf::from(base).join("Skyrim Special Edition"))
    }

    /// Directory the game reads Plugins.txt from: its local app data folder inside the
    /// Proton prefix.
    #[cfg(not(windows))]
    pub fn local_appdata_dir() -> Result<PathBuf> {
        let user = wine_user_dir().ok_or_else(|| {
            anyhow!("Couldn't find Skyrim's Proton prefix. Start the game once from Steam first.")
        })?;
        Ok(user.join("AppData").join("Local").join("Skyrim Special Edition"))
    }

    pub fn plugins_txt() -> Result<PathBuf> {
        Ok(Self::local_appdata_dir()?.join("Plugins.txt"))
    }

    /// Creation Club plugins listed in Skyrim.ccc are always active.
    fn ccc_plugins(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("Skyrim.ccc"))
            .map(|s| {
                s.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    }

}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
fn registry_paths() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn registry_paths() -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::*;
    let mut out = Vec::new();
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    for key in [
        r"SOFTWARE\WOW6432Node\Bethesda Softworks\Skyrim Special Edition",
        r"SOFTWARE\Bethesda Softworks\Skyrim Special Edition",
    ] {
        if let Ok(k) = hklm.open_subkey(key)
            && let Ok(p) = k.get_value::<String, _>("installed path")
        {
            out.push(PathBuf::from(p));
        }
    }
    out
}

/// Where Steam itself is installed.
#[cfg(windows)]
fn steam_roots() -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::*;
    let mut roots = Vec::new();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(k) = hkcu.open_subkey(r"Software\Valve\Steam")
        && let Ok(p) = k.get_value::<String, _>("SteamPath")
    {
        roots.push(PathBuf::from(p));
    }
    roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    roots
}

/// Where Steam itself is installed: the native, Flatpak and Snap locations on Linux, and
/// the macOS one.
#[cfg(not(windows))]
fn steam_roots() -> Vec<PathBuf> {
    let Some(dirs) = directories::UserDirs::new() else { return Vec::new() };
    let home = dirs.home_dir();
    [
        ".local/share/Steam",
        ".steam/steam",
        ".steam/root",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        "snap/steam/common/.local/share/Steam",
        "Library/Application Support/Steam",
    ]
    .iter()
    .map(|p| home.join(p))
    .filter(|p| p.is_dir())
    .collect()
}

/// Every Steam library folder (each holds a `steamapps` folder).
fn steam_libraries() -> Vec<PathBuf> {
    let mut libs = Vec::new();
    for root in steam_roots() {
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = fs::read_to_string(&vdf) {
            for line in text.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("\"path\"") {
                    let p = rest.trim().trim_matches('"').replace("\\\\", "\\");
                    libs.push(PathBuf::from(p));
                }
            }
        }
        libs.push(root);
    }
    libs
}

fn steam_library_paths() -> Vec<PathBuf> {
    steam_libraries()
        .into_iter()
        .map(|l| l.join("steamapps").join("common").join("Skyrim Special Edition"))
        .collect()
}

/// The Windows user folder inside Skyrim's Proton prefix, where the game keeps what it
/// would put in the user's own folders on Windows (Plugins.txt, saves, SKSE logs).
#[cfg(not(windows))]
pub fn wine_user_dir() -> Option<PathBuf> {
    steam_libraries()
        .into_iter()
        .map(|l| {
            l.join("steamapps")
                .join("compatdata")
                .join(STEAM_APP_ID)
                .join("pfx")
                .join("drive_c")
                .join("users")
                .join("steamuser")
        })
        .find(|p| p.is_dir())
}

#[cfg(not(windows))]
fn common_paths() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn common_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for drive in ["C", "D", "E", "F", "G"] {
        for tail in [
            r"Program Files (x86)\Steam\steamapps\common\Skyrim Special Edition",
            r"Steam\steamapps\common\Skyrim Special Edition",
            r"SteamLibrary\steamapps\common\Skyrim Special Edition",
            r"Games\Steam\steamapps\common\Skyrim Special Edition",
        ] {
            out.push(PathBuf::from(format!("{drive}:\\{tail}")));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Plugins
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginKind {
    Master,
    Light,
    Normal,
}

impl PluginKind {
    pub fn label(self) -> &'static str {
        match self {
            PluginKind::Master => "ESM",
            PluginKind::Light => "ESL",
            PluginKind::Normal => "ESP",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Plugin {
    pub name: String,
    pub kind: PluginKind,
    pub masters: Vec<String>,
    pub enabled: bool,
    /// Ships with the game; always loaded and can't be toggled.
    pub core: bool,
    pub missing_masters: Vec<String>,
}

pub fn is_plugin_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl")
}

/// Whether this plugin ships with the game itself.
pub fn is_core_plugin(name: &str) -> bool {
    CORE_PLUGINS.iter().any(|c| c.eq_ignore_ascii_case(name))
}

/// The plugins this plugin is built on, as named in its header. Empty if it can't be read.
pub fn plugin_masters(path: &Path) -> Vec<String> {
    read_header(path).map(|(_, masters)| masters).unwrap_or_default()
}

/// Reads the TES4 header of a plugin to learn its flags and masters.
fn read_header(path: &Path) -> Result<(u32, Vec<String>)> {
    let mut f = fs::File::open(path)?;
    let mut head = [0u8; 24];
    f.read_exact(&mut head)?;
    if &head[0..4] != b"TES4" {
        bail!("not a plugin");
    }
    let data_size = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
    let flags = u32::from_le_bytes(head[8..12].try_into().unwrap());

    let mut data = vec![0u8; data_size.min(4 * 1024 * 1024)];
    let n = f.read(&mut data)?;
    data.truncate(n);

    let mut masters = Vec::new();
    let mut i = 0;
    while i + 6 <= data.len() {
        let ty = &data[i..i + 4];
        let size = u16::from_le_bytes([data[i + 4], data[i + 5]]) as usize;
        let start = i + 6;
        let end = (start + size).min(data.len());
        if ty == b"MAST" {
            let s = String::from_utf8_lossy(&data[start..end]);
            masters.push(s.trim_end_matches('\0').to_string());
        }
        i = end;
    }
    Ok((flags, masters))
}

fn classify(name: &str, flags: u32) -> PluginKind {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".esl") || flags & 0x200 != 0 {
        PluginKind::Light
    } else if lower.ends_with(".esm") || flags & 0x1 != 0 {
        PluginKind::Master
    } else {
        PluginKind::Normal
    }
}

/// Plugins.txt: one plugin per line, `*` prefix means enabled.
pub fn read_plugins_txt() -> Vec<(String, bool)> {
    let Ok(path) = Game::plugins_txt() else {
        return Vec::new();
    };
    fs::read_to_string(path)
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| match l.strip_prefix('*') {
                    Some(rest) => (rest.to_string(), true),
                    None => (l.to_string(), false),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn write_plugins_txt(plugins: &[Plugin]) -> Result<()> {
    let path = Game::plugins_txt()?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut text = String::from(
        "# This file is used by Skyrim to keep track of your downloaded content.\n# Managed by Skyrim Mod Manager.\n",
    );
    for p in plugins.iter().filter(|p| !p.core) {
        if p.enabled {
            text.push('*');
        }
        text.push_str(&p.name);
        text.push('\n');
    }
    fs::write(path, text)?;
    Ok(())
}

/// Scans the Data folder and merges with the saved load order.
pub fn scan_plugins(game: &Game) -> Vec<Plugin> {
    let data = game.data_dir();
    let saved = read_plugins_txt();
    let ccc = game.ccc_plugins();

    let mut found: Vec<Plugin> = Vec::new();
    if let Ok(rd) = fs::read_dir(&data) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !is_plugin_file(&name) || !entry.path().is_file() {
                continue;
            }
            let (flags, masters) = read_header(&entry.path()).unwrap_or((0, Vec::new()));
            let core = is_core_plugin(&name) || ccc.iter().any(|c| c.eq_ignore_ascii_case(&name));
            found.push(Plugin {
                kind: classify(&name, flags),
                masters,
                enabled: core,
                core,
                missing_masters: Vec::new(),
                name,
            });
        }
    }

    // Order: core plugins first (in the game's own order), then the saved order, then anything new.
    let mut ordered: Vec<Plugin> = Vec::new();
    let take = |name: &str, found: &mut Vec<Plugin>| -> Option<Plugin> {
        let idx = found.iter().position(|p| p.name.eq_ignore_ascii_case(name))?;
        Some(found.remove(idx))
    };
    for core in CORE_PLUGINS.iter().map(|s| s.to_string()).chain(ccc.iter().cloned()) {
        if let Some(p) = take(&core, &mut found) {
            ordered.push(p);
        }
    }
    for (name, enabled) in &saved {
        if let Some(mut p) = take(name, &mut found) {
            p.enabled = *enabled || p.core;
            ordered.push(p);
        }
    }
    // New plugins: masters before the rest, enabled by default.
    found.sort_by_key(|p| (p.kind != PluginKind::Master, p.name.to_ascii_lowercase()));
    for mut p in found {
        p.enabled = true;
        ordered.push(p);
    }

    refresh_missing_masters(&mut ordered);
    ordered
}

pub fn refresh_missing_masters(plugins: &mut [Plugin]) {
    let present: Vec<String> = plugins
        .iter()
        .filter(|p| p.enabled)
        .map(|p| p.name.to_ascii_lowercase())
        .collect();
    for p in plugins.iter_mut() {
        p.missing_masters = p
            .masters
            .iter()
            .filter(|m| !present.contains(&m.to_ascii_lowercase()))
            .cloned()
            .collect();
    }
}
