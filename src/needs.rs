//! What a mod needs before it works, read from the mod's own files.
//!
//! Nexus lists a mod's requirements on its page but not through its API, so this works
//! them out locally instead: a plugin names the plugins it's built on in its header, and an
//! SKSE plugin DLL needs the script extender — and nearly always Address Library, which it
//! names inside the binary when it does.

use crate::game;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

/// Nexus mod id of Address Library for SKSE Plugins.
pub const ADDRESS_LIBRARY_MOD_ID: u64 = 32444;
pub const ADDRESS_LIBRARY_NAME: &str = "Address Library";
pub const SKSE_NAME: &str = "SKSE";

/// The largest DLL we bother reading through for markers.
const MAX_DLL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    /// Plugins this mod's plugins are built on, beyond the game's own ("SkyUI_SE.esp").
    pub masters: Vec<String>,
    /// Ships an SKSE plugin, so the script extender must be installed.
    pub skse: bool,
    /// One of its SKSE plugins reads Address Library's offsets file.
    pub address_library: bool,
    /// Address Library offsets files this mod itself provides ("versionlib-1-7-104-0.bin"), lowercase.
    pub versionlibs: Vec<String>,
    /// What each SKSE plugin DLL says about the game versions it was built for.
    pub plugins: Vec<PluginVersion>,
}

/// An SKSE plugin's own account of which game versions it works with — the
/// `SKSEPlugin_Version` data the script extender reads before loading it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginVersion {
    pub dll: String,
    /// Exports the version data at all; plugins from before the Anniversary update don't.
    pub has_data: bool,
    /// `versionIndependence` flags: finds addresses via Address Library or signatures.
    pub independence: u32,
    /// `versionIndependenceEx` flags: struct-free, or ready for the 1.7.99+ Address Library format.
    pub independence_ex: u32,
    /// Packed game versions it was built against, when it names them.
    pub compatible: Vec<u32>,
    /// When the DLL was linked (Unix time, from its PE header).
    pub build_time: u32,
    /// Built on a CommonLibSSE from before the 1.7.99 offsets format — it carries that
    /// library's old offsets-database source names, which newer ones dropped.
    pub old_commonlib: bool,
}

const INDEP_ADDRESS_LIBRARY: u32 = 1 << 0;
const INDEP_SIGNATURES: u32 = 1 << 1;
const INDEP_STRUCTS_POST_629: u32 = 1 << 2;
const INDEP_EX_NO_STRUCTS: u32 = 1 << 0;
const INDEP_EX_ADDRESS_LIBRARY_V5: u32 = 1 << 1;
/// Versions where the game changed under plugins: struct layouts, then the offsets format.
const GAME_1_6_629: u32 = pack_version(1, 6, 629, 0);
const GAME_1_7_99: u32 = pack_version(1, 7, 99, 0);
/// SKSE refuses Address Library plugins built before commonlib knew the new format
/// (2025-05-26) unless they name the game version; later builds are let through and fail
/// on their own when they open the offsets file.
const NEW_FORMAT_BUILD_TIME: u32 = 1_748_217_600;

/// A plugin that won't work with the installed game, in the user's terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outdated {
    pub dll: String,
    /// "1.6.1170" or "before 1.7.99" — the Skyrim it was made for.
    pub made_for: String,
    /// SKSE will refuse to load it (else it loads and usually fails on its own).
    pub certain: bool,
}

impl PluginVersion {
    /// Whether this plugin fits the installed game, following the checks SKSE itself makes.
    pub fn fit(&self, game: u32) -> Option<Outdated> {
        let outdated = |made_for: String, certain: bool| Some(Outdated { dll: self.dll.clone(), made_for, certain });
        if !self.has_data {
            return outdated("1.5.97".into(), true);
        }
        if self.compatible.contains(&game) {
            return None;
        }
        // Flags we don't know are from a newer script extender; leave the judgement to it.
        if self.independence & !(INDEP_ADDRESS_LIBRARY | INDEP_SIGNATURES | INDEP_STRUCTS_POST_629) != 0 {
            return None;
        }
        let independent = self.independence & (INDEP_ADDRESS_LIBRARY | INDEP_SIGNATURES) != 0;
        if self.independence & INDEP_ADDRESS_LIBRARY != 0
            && self.independence_ex & INDEP_EX_ADDRESS_LIBRARY_V5 == 0
            && game >= GAME_1_7_99
        {
            // SKSE turns away old builds; newer ones without the flag get in, and fail on
            // their own if their CommonLib predates the format (some just forgot the flag).
            let made_for = match self.compatible.iter().max() {
                Some(&v) => format_version(v),
                None => "before 1.7.99".into(),
            };
            if self.build_time < NEW_FORMAT_BUILD_TIME {
                return outdated(made_for, true);
            }
            if self.old_commonlib {
                return outdated(made_for, false);
            }
        }
        if independent
            && game >= GAME_1_6_629
            && self.independence & INDEP_STRUCTS_POST_629 == 0
            && self.independence_ex & INDEP_EX_NO_STRUCTS == 0
        {
            return outdated("before 1.6.629".into(), true);
        }
        if independent {
            return None;
        }
        let made_for = match self.compatible.iter().max() {
            Some(&v) => format_version(v),
            None => "an older Skyrim".into(),
        };
        outdated(made_for, true)
    }
}

/// SKSE's packing of a game version: "1.7.104.0" → 0x01070680.
pub const fn pack_version(major: u32, minor: u32, build: u32, sub: u32) -> u32 {
    ((major & 0xff) << 24) | ((minor & 0xff) << 16) | ((build & 0xfff) << 4) | (sub & 0xf)
}

/// "1.7.104.0" as SKSE packs it, when it's a version we can read.
pub fn parse_version(s: &str) -> Option<u32> {
    let mut parts = s.split('.').map(|p| p.trim().parse::<u32>().ok());
    let major = parts.next()??;
    let minor = parts.next()??;
    let build = parts.next()??;
    let sub = parts.next().unwrap_or(Some(0))?;
    Some(pack_version(major, minor, build, sub))
}

/// 0x01060492 → "1.6.1170" (the trailing zero most versions carry is left off).
pub fn format_version(v: u32) -> String {
    let (major, minor, build, sub) = (v >> 24, (v >> 16) & 0xff, (v >> 4) & 0xfff, v & 0xf);
    if sub == 0 { format!("{major}.{minor}.{build}") } else { format!("{major}.{minor}.{build}.{sub}") }
}

/// Looks through a stored mod for everything it depends on.
pub fn scan(mod_dir: &Path) -> Needs {
    let mut needs = Needs::default();
    for entry in WalkDir::new(mod_dir).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if game::is_plugin_file(&name) {
            for m in game::plugin_masters(entry.path()) {
                if !game::is_core_plugin(&m) && !needs.masters.iter().any(|x| x.eq_ignore_ascii_case(&m)) {
                    needs.masters.push(m);
                }
            }
        } else if in_skse_plugins(entry.path(), mod_dir) {
            if lower.ends_with(".dll") {
                let (skse, address_library, version) = dll_markers(entry.path());
                needs.skse |= skse;
                needs.address_library |= address_library;
                if let Some(mut v) = version {
                    v.dll = name.clone();
                    needs.plugins.push(v);
                }
            } else if lower.starts_with("versionlib-") && lower.ends_with(".bin") {
                needs.versionlibs.push(lower);
            }
        }
    }
    needs.masters.sort_by_key(|m| m.to_ascii_lowercase());
    needs
}

/// Whether a file sits in `SKSE\Plugins` (relative to the mod's Data root).
fn in_skse_plugins(path: &Path, mod_dir: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(mod_dir) else { return false };
    let mut parts = rel.iter().map(|s| s.to_string_lossy().to_ascii_lowercase());
    parts.next() == Some("skse".into()) && parts.next() == Some("plugins".into())
}

/// (is an SKSE plugin, loads Address Library, its version data) — from the strings the DLL
/// carries: SKSE plugins export `SKSEPlugin_Load` / `SKSEPlugin_Version`, and
/// CommonLib-based ones open `versionlib-*.bin` by name.
fn dll_markers(path: &Path) -> (bool, bool, Option<PluginVersion>) {
    if fs::metadata(path).map(|m| m.len()).unwrap_or(u64::MAX) > MAX_DLL_BYTES {
        return (false, false, None);
    }
    let Ok(bytes) = fs::read(path) else { return (false, false, None) };
    let skse = contains(&bytes, b"SKSEPlugin_Load") || contains(&bytes, b"SKSEPlugin_Version");
    let versionlib = skse && (contains(&bytes, b"versionlib") || contains_utf16(&bytes, "versionlib"));
    let version = skse.then(|| plugin_version(&bytes)).flatten().map(|mut v| {
        v.old_commonlib = contains(&bytes, b"REL/ID.h") || contains(&bytes, b"IDDatabase");
        v
    });
    (skse, versionlib, version)
}

/// Reads an SKSE plugin's `SKSEPlugin_Version` export out of the DLL's export table.
/// `None` when it isn't a plugin (exports none of SKSE's entry points) or the file is odd.
fn plugin_version(bytes: &[u8]) -> Option<PluginVersion> {
    let pe = Pe::parse(bytes)?;
    let mut version = PluginVersion { build_time: pe.build_time, ..Default::default() };
    let mut is_plugin = false;
    for (name, rva) in pe.exports() {
        match name {
            "SKSEPlugin_Load" | "SKSEPlugin_Query" => is_plugin = true,
            "SKSEPlugin_Version" => {
                is_plugin = true;
                let at = pe.offset(rva)?;
                // dataVersion, pluginVersion, name[256], author[256], supportEmail[252],
                // versionIndependenceEx, versionIndependence, compatibleVersions[16], seVersionRequired.
                let flags = at + 8 + 256 + 256 + 252;
                version.has_data = true;
                version.independence_ex = u32_at(bytes, flags)?;
                version.independence = u32_at(bytes, flags + 4)?;
                for i in 0..16 {
                    match u32_at(bytes, flags + 8 + i * 4)? {
                        0 => break,
                        v => version.compatible.push(v),
                    }
                }
            }
            _ => {}
        }
    }
    is_plugin.then_some(version)
}

/// Just enough of a Windows executable to find its exports.
struct Pe<'a> {
    bytes: &'a [u8],
    build_time: u32,
    /// (virtual address, size, file offset) per section.
    sections: Vec<(u32, u32, u32)>,
    export_dir: u32,
}

impl<'a> Pe<'a> {
    fn parse(bytes: &'a [u8]) -> Option<Self> {
        let pe = u32_at(bytes, 0x3c)? as usize;
        if bytes.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let coff = pe + 4;
        let section_count = u16_at(bytes, coff + 2)? as usize;
        let build_time = u32_at(bytes, coff + 4)?;
        let optional_size = u16_at(bytes, coff + 16)? as usize;
        let optional = coff + 20;
        // The data directories sit later in a 64-bit optional header.
        let directories = optional + if u16_at(bytes, optional)? == 0x20b { 112 } else { 96 };
        let export_dir = u32_at(bytes, directories)?;
        let mut sections = Vec::with_capacity(section_count);
        for i in 0..section_count {
            let s = optional + optional_size + i * 40;
            let virtual_size = u32_at(bytes, s + 8)?;
            let raw_size = u32_at(bytes, s + 16)?;
            sections.push((u32_at(bytes, s + 12)?, virtual_size.max(raw_size), u32_at(bytes, s + 20)?));
        }
        Some(Pe { bytes, build_time, sections, export_dir })
    }

    /// Where a virtual address lives in the file.
    fn offset(&self, rva: u32) -> Option<usize> {
        self.sections
            .iter()
            .find(|(start, size, _)| rva >= *start && rva < start.saturating_add(*size))
            .map(|(start, _, raw)| (rva - start + raw) as usize)
    }

    /// Every named export with its virtual address.
    fn exports(&self) -> Vec<(&'a str, u32)> {
        let mut out = Vec::new();
        let Some(dir) = (self.export_dir != 0).then(|| self.offset(self.export_dir)).flatten() else { return out };
        let read = |at: usize| u32_at(self.bytes, at);
        let (Some(name_count), Some(functions), Some(names), Some(ordinals)) =
            (read(dir + 24), read(dir + 28), read(dir + 32), read(dir + 36))
        else {
            return out;
        };
        let (Some(functions), Some(names), Some(ordinals)) = (self.offset(functions), self.offset(names), self.offset(ordinals))
        else {
            return out;
        };
        for i in 0..name_count.min(4096) as usize {
            let Some(name_at) = read(names + i * 4).and_then(|rva| self.offset(rva)) else { continue };
            let Some(rest) = self.bytes.get(name_at..) else { continue };
            let len = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
            let Ok(name) = std::str::from_utf8(&rest[..len]) else { continue };
            let Some(ordinal) = u16_at(self.bytes, ordinals + i * 2) else { continue };
            if let Some(rva) = read(functions + ordinal as usize * 4) {
                out.push((name, rva));
            }
        }
        out
    }
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn contains_utf16(hay: &[u8], needle: &str) -> bool {
    let wide: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
    contains(hay, &wide)
}

/// "SkyUI_SE.esp" → "SkyUI SE", for showing a master by something like a name.
pub fn display_name(plugin: &str) -> String {
    let stem = plugin.rsplit_once('.').map_or(plugin, |(s, _)| s);
    stem.replace('_', " ")
}

/// The offsets file Address Library must provide for this game version ("1.7.104.0").
pub fn versionlib_for(game_version: &str) -> String {
    format!("versionlib-{}.bin", game_version.replace('.', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_in_bytes() {
        let dll = b"\x00junk SKSEPlugin_Load more versionlib-%d-%d-%d-%d.bin\x00";
        assert!(contains(dll, b"SKSEPlugin_Load"));
        assert!(contains(dll, b"versionlib"));
        assert!(!contains(dll, b"Trainwreck"));
        let wide: Vec<u8> = "abc versionlib".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(contains_utf16(&wide, "versionlib"));
    }

    #[test]
    fn names() {
        assert_eq!(display_name("SkyUI_SE.esp"), "SkyUI SE");
        assert_eq!(display_name("Dawnguard.esm"), "Dawnguard");
        assert_eq!(versionlib_for("1.7.104.0"), "versionlib-1-7-104-0.bin");
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("1.7.104.0"), Some(0x0107_0680));
        assert_eq!(parse_version("1.6.1170"), Some(0x0106_4920));
        assert_eq!(parse_version("nope"), None);
        assert_eq!(format_version(0x0106_4920), "1.6.1170");
        assert_eq!(format_version(0x0105_0611), "1.5.97.1");
    }

    #[test]
    fn plugin_fit() {
        let game = parse_version("1.7.104.0").unwrap();
        let old_game = parse_version("1.6.1170.0").unwrap();
        let recent = NEW_FORMAT_BUILD_TIME + 1;
        let made = |v: PluginVersion| v.fit(game).map(|o| (o.made_for, o.certain));

        // Names the game version — fine whatever else it says.
        let listed = PluginVersion { has_data: true, independence: 5, independence_ex: 2, compatible: vec![game], ..Default::default() };
        assert_eq!(made(listed.clone()), None);
        // Hard-coded for last year's game only.
        let for_old = PluginVersion { independence: 0, compatible: vec![old_game], ..listed.clone() };
        assert_eq!(made(for_old.clone()), Some(("1.6.1170".into(), true)));
        assert_eq!(for_old.fit(old_game), None);
        // Version-independent through Address Library, ready for the new format.
        let modern = PluginVersion { compatible: vec![], ..listed.clone() };
        assert_eq!(made(modern.clone()), None);
        // Same, but on a CommonLib from before the offsets format changed: SKSE lets it in
        // and it fails on its own.
        let stale = PluginVersion { independence_ex: 1, build_time: recent, old_commonlib: true, ..modern.clone() };
        assert_eq!(made(stale.clone()), Some(("before 1.7.99".into(), false)));
        assert_eq!(made(PluginVersion { build_time: 0, ..stale.clone() }), Some(("before 1.7.99".into(), true)));
        assert_eq!(stale.fit(old_game), None);
        // A recent build that merely forgot the flag is left alone.
        assert_eq!(made(PluginVersion { old_commonlib: false, ..stale.clone() }), None);
        // Doesn't know about the 1.6.629 structs and isn't struct-free.
        let pre_629 = PluginVersion { independence: 1, independence_ex: 2, ..modern.clone() };
        assert_eq!(made(pre_629), Some(("before 1.6.629".into(), true)));
        // No version data at all: from before the Anniversary update.
        assert_eq!(made(PluginVersion::default()), Some(("1.5.97".into(), true)));
        // Flags from a newer SKSE than we know: leave it alone.
        assert_eq!(made(PluginVersion { independence: 8, ..listed.clone() }), None);
    }

    #[test]
    fn reads_exports() {
        // A minimal 64-bit PE with one section holding an export table for
        // SKSEPlugin_Version, whose data lists game 1.6.1170.
        let mut file = vec![0u8; 0x400];
        file[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        file[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        file[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        file[coff + 4..coff + 8].copy_from_slice(&123u32.to_le_bytes());
        file[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
        let opt = coff + 20;
        file[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        // Export directory at RVA 0x1000, mapped to file offset 0x200.
        file[opt + 112..opt + 116].copy_from_slice(&0x1000u32.to_le_bytes());
        let sec = opt + 240;
        file[sec + 8..sec + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        file[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        file[sec + 16..sec + 20].copy_from_slice(&0x200u32.to_le_bytes());
        file[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes());
        let dir = 0x200;
        let put = |file: &mut Vec<u8>, at: usize, v: u32| file[at..at + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut file, dir + 20, 1); // functions
        put(&mut file, dir + 24, 1); // names
        put(&mut file, dir + 28, 0x1040); // AddressOfFunctions
        put(&mut file, dir + 32, 0x1050); // AddressOfNames
        put(&mut file, dir + 36, 0x1060); // AddressOfNameOrdinals
        put(&mut file, 0x240, 0x1100); // function 0 → version data at RVA 0x1100
        put(&mut file, 0x250, 0x1070); // name 0 at RVA 0x1070
        file[0x260..0x262].copy_from_slice(&0u16.to_le_bytes());
        file[0x270..0x270 + 18].copy_from_slice(b"SKSEPlugin_Version");
        file.resize(0x300 + 848, 0);
        let data = 0x300;
        put(&mut file, data, 1);
        put(&mut file, data + 772, 2);
        put(&mut file, data + 776, 5);
        put(&mut file, data + 780, 0x0106_4920);
        let v = plugin_version(&file).expect("a plugin");
        assert!(v.has_data);
        assert_eq!((v.independence_ex, v.independence, v.build_time), (2, 5, 123));
        assert_eq!(v.compatible, vec![0x0106_4920]);
        assert!(plugin_version(b"MZ not a real file").is_none());
    }

    #[test]
    fn skse_plugins_folder() {
        let root = Path::new("C:/mods/x");
        assert!(in_skse_plugins(Path::new("C:/mods/x/SKSE/Plugins/a.dll"), root));
        assert!(!in_skse_plugins(Path::new("C:/mods/x/Scripts/a.dll"), root));
        assert!(!in_skse_plugins(Path::new("C:/mods/x/_root/skse64_loader.exe"), root));
    }
}
