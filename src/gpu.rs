//! Graphics card, driver and DLSS runtime detection for the Settings page.
// Graphics card detection reads the Windows registry, so its helpers go unused elsewhere.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::game::Game;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use winreg::RegKey;
#[cfg(windows)]
use winreg::enums::HKEY_LOCAL_MACHINE;

/// Minimum GeForce driver for DLSS 5 (Game Ready 595.79, September 2026).
pub const DLSS5_MIN_DRIVER: (u32, u32) = (595, 79);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

#[derive(Clone, Debug)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: Vendor,
    /// Driver version as the vendor shows it, e.g. "595.79" for GeForce.
    pub driver: String,
    /// GeForce driver as (major, minor), when it could be parsed.
    geforce: Option<(u32, u32)>,
}

impl GpuInfo {
    /// DLSS 5 needs an RTX 50-series card. Newer cards keep the "RTX" prefix, so the check
    /// is by generation number rather than an explicit list.
    pub fn rtx_generation(&self) -> Option<u32> {
        if self.vendor != Vendor::Nvidia {
            return None;
        }
        let upper = self.name.to_ascii_uppercase();
        let rest = upper.split("RTX").nth(1)?;
        let digits: String = rest.trim_start().chars().take_while(char::is_ascii_digit).collect();
        let model: u32 = digits.parse().ok()?;
        Some(model / 1000 * 10)
    }

    pub fn driver_supports_dlss5(&self) -> Option<bool> {
        self.geforce.map(|v| v >= DLSS5_MIN_DRIVER)
    }
}

fn vendor_of(provider: &str, desc: &str) -> Vendor {
    let s = format!("{provider} {desc}").to_ascii_lowercase();
    if s.contains("nvidia") || s.contains("geforce") {
        Vendor::Nvidia
    } else if s.contains("amd") || s.contains("radeon") || s.contains("advanced micro") {
        Vendor::Amd
    } else if s.contains("intel") {
        Vendor::Intel
    } else {
        Vendor::Other
    }
}

/// Windows stores the GeForce driver as e.g. "32.0.15.9579": the last five digits of the
/// final two parts spell out the marketing version, 595.79.
fn geforce_version(windows_version: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = windows_version.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let joined = format!("{}{}", parts[2], parts[3]);
    if joined.len() < 5 {
        return None;
    }
    let tail = &joined[joined.len() - 5..];
    let major = tail[..3].parse().ok()?;
    let minor = tail[3..].parse().ok()?;
    Some((major, minor))
}

/// Graphics card detection is only implemented for Windows.
#[cfg(not(windows))]
pub fn detect() -> Option<GpuInfo> {
    None
}

/// Reads the display adapters from the registry (works without loading any vendor DLL).
/// Prefers a discrete NVIDIA/AMD card over an integrated Intel one.
#[cfg(windows)]
pub fn detect() -> Option<GpuInfo> {
    const CLASS: &str =
        r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let class = hklm.open_subkey(CLASS).ok()?;
    let mut found: Vec<GpuInfo> = Vec::new();
    for sub in class.enum_keys().flatten() {
        if !sub.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(key) = class.open_subkey(&sub) else { continue };
        let Ok(name) = key.get_value::<String, _>("DriverDesc") else { continue };
        let version: String = key.get_value("DriverVersion").unwrap_or_default();
        let provider: String = key.get_value("ProviderName").unwrap_or_default();
        let vendor = vendor_of(&provider, &name);
        let geforce = if vendor == Vendor::Nvidia { geforce_version(&version) } else { None };
        let driver = match geforce {
            Some((a, b)) => format!("{a}.{b:02}"),
            None => version,
        };
        found.push(GpuInfo { name, vendor, driver, geforce });
    }
    let rank = |v: &Vendor| match v {
        Vendor::Nvidia | Vendor::Amd => 0,
        Vendor::Intel => 1,
        Vendor::Other => 2,
    };
    found.sort_by_key(|g| rank(&g.vendor));
    found.into_iter().next()
}

/// A DLSS component found in the game folder.
#[derive(Clone, Debug)]
pub struct Runtime {
    pub label: &'static str,
    pub file: &'static str,
    pub version: Option<String>,
}

/// The NVIDIA runtimes the Skyrim upscaler mods drop next to SkyrimSE.exe.
const RUNTIMES: &[(&str, &str)] = &[
    ("Super Resolution", "nvngx_dlss.dll"),
    ("Frame Generation", "nvngx_dlssg.dll"),
    ("Ray Reconstruction", "nvngx_dlssd.dll"),
    ("Neural Rendering (DLSS 5)", "nvngx_dlssnr.dll"),
];

/// Where the upscaler mods keep their runtimes, relative to the game root: next to the
/// exe (Skyrim Upscaler, OptiScaler), Streamline folders (Open Shaders / Community
/// Shaders) or the Upscaler base plugin folder (PureDark's newer builds).
#[cfg(windows)]
const RUNTIME_DIRS: &[&str] = &[
    "",
    r"Data\Shaders\Upscaling\Streamline",
    r"Data\Shaders\Upscaling\StreamlineDX12",
    r"Data\SKSE\Plugins\UpscalerBasePlugin",
];
#[cfg(not(windows))]
const RUNTIME_DIRS: &[&str] = &[
    "",
    "Data/Shaders/Upscaling/Streamline",
    "Data/Shaders/Upscaling/StreamlineDX12",
    "Data/SKSE/Plugins/UpscalerBasePlugin",
];

pub fn installed_runtimes(game: &Game) -> Vec<Runtime> {
    RUNTIMES
        .iter()
        .filter_map(|(label, file)| {
            let path = RUNTIME_DIRS.iter().map(|d| game.root.join(d).join(file)).find(|p| p.is_file())?;
            Some(Runtime { label, file, version: file_version(&path) })
        })
        .collect()
}

/// Friendly name for a DLSS runtime DLL, e.g. "nvngx_dlss.dll" -> "DLSS Super Resolution".
pub fn runtime_label(file_name: &str) -> Option<String> {
    let lower = file_name.to_ascii_lowercase();
    RUNTIMES
        .iter()
        .find(|(_, f)| *f == lower)
        .map(|(label, _)| format!("DLSS {label}"))
}

/// Where a loose DLSS runtime dropped by the user should live, relative to Data, based on
/// the upscaler mod currently in the game folder. `None` means next to the exe.
pub fn runtime_home(game: &Game, file_name: &str) -> Option<PathBuf> {
    let lower = file_name.to_ascii_lowercase();
    if !RUNTIMES.iter().any(|(_, f)| *f == lower) {
        return None;
    }
    // Open Shaders keeps the DX12-only Frame Generation runtime in its own Streamline folder.
    let preferred: &[&str] = if lower == "nvngx_dlssg.dll" {
        &[RUNTIME_DIRS[2], RUNTIME_DIRS[1], RUNTIME_DIRS[3]]
    } else {
        &[RUNTIME_DIRS[1], RUNTIME_DIRS[3]]
    };
    preferred
        .iter()
        .find(|d| game.root.join(d).is_dir())
        .and_then(|d| Path::new(d).strip_prefix("Data").ok())
        .map(Path::to_path_buf)
}

/// Product version from a DLL's version resource, e.g. "310.8.0.0".
#[cfg(not(windows))]
pub fn file_version(path: &Path) -> Option<String> {
    fixed_file_info_version(&std::fs::read(path).ok()?)
}

/// Reads the product version straight out of a Windows executable's bytes, for systems
/// without the Win32 version API. The version resource holds a VS_FIXEDFILEINFO block that
/// starts with a fixed signature and structure version, followed by the file and product
/// versions as pairs of 32-bit numbers.
#[cfg(any(not(windows), test))]
fn fixed_file_info_version(bytes: &[u8]) -> Option<String> {
    const HEADER: [u8; 8] = [0xBD, 0x04, 0xEF, 0xFE, 0x00, 0x00, 0x01, 0x00];
    let at = bytes.windows(HEADER.len()).position(|w| w == HEADER)?;
    let field = |i: usize| {
        let b = bytes.get(at + i * 4..at + i * 4 + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (ms, ls) = (field(4)?, field(5)?);
    Some(format!("{}.{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16, ls & 0xffff))
}

/// Product version from a DLL's version resource, e.g. "310.8.0.0".
#[cfg(windows)]
pub fn file_version(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    };

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: plain Win32 calls with buffers we own; sizes come from the API itself.
    unsafe {
        let mut handle = 0;
        let size = GetFileVersionInfoSizeW(wide.as_ptr(), &mut handle);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(wide.as_ptr(), 0, size, buf.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let sub: Vec<u16> = "\\".encode_utf16().chain(std::iter::once(0)).collect();
        let mut info: *mut VS_FIXEDFILEINFO = std::ptr::null_mut();
        let mut len = 0u32;
        let ok = VerQueryValueW(
            buf.as_ptr().cast(),
            sub.as_ptr(),
            (&mut info as *mut *mut VS_FIXEDFILEINFO).cast(),
            &mut len,
        );
        if ok == 0 || info.is_null() || (len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>() {
            return None;
        }
        let i = &*info;
        Some(format!(
            "{}.{}.{}.{}",
            i.dwProductVersionMS >> 16,
            i.dwProductVersionMS & 0xffff,
            i.dwProductVersionLS >> 16,
            i.dwProductVersionLS & 0xffff
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geforce_versions() {
        assert_eq!(geforce_version("32.0.15.9579"), Some((595, 79)));
        assert_eq!(geforce_version("31.0.15.5222"), Some((552, 22)));
        assert_eq!(geforce_version("30.0.14.7168"), Some((471, 68)));
        assert_eq!(geforce_version("1.2.3"), None);
    }

    #[test]
    fn version_from_raw_bytes() {
        let mut bytes = vec![0u8; 37];
        for field in [0xFEEF_04BDu32, 0x0001_0000, 0x0001_0006, 0x0491_0000, 0x0001_0007, 0x0068_0000] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        bytes.extend_from_slice(&[0; 9]);
        assert_eq!(fixed_file_info_version(&bytes).as_deref(), Some("1.7.104.0"));
        assert_eq!(fixed_file_info_version(&bytes[..50]), None);
        assert_eq!(fixed_file_info_version(b"not an executable"), None);
    }

    /// The byte scan must agree with what Windows itself reports.
    #[cfg(windows)]
    #[test]
    fn raw_bytes_match_windows_api() {
        let dll = Path::new(r"C:\Windows\System32\kernel32.dll");
        let Ok(bytes) = std::fs::read(dll) else { return };
        assert_eq!(fixed_file_info_version(&bytes), file_version(dll));
    }

    #[test]
    fn rtx_generations() {
        let gpu = |name: &str| GpuInfo {
            name: name.into(),
            vendor: Vendor::Nvidia,
            driver: String::new(),
            geforce: None,
        };
        assert_eq!(gpu("NVIDIA GeForce RTX 5070 Ti").rtx_generation(), Some(50));
        assert_eq!(gpu("NVIDIA GeForce RTX 4090").rtx_generation(), Some(40));
        assert_eq!(gpu("NVIDIA GeForce GTX 1080").rtx_generation(), None);
    }
}
