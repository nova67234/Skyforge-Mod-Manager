//! Crash reports Skyrim leaves behind when it dies.
//!
//! The game itself records nothing. With Crash Logger (or Trainwreck) installed, every
//! crash writes a text report to `Documents\My Games\Skyrim Special Edition\SKSE`; this
//! finds those reports and boils one down to "what went wrong and which files were involved".

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Nexus mod id of Crash Logger SSE/AE/VR, the report writer we install.
pub const LOGGER_MOD_ID: u64 = 59818;
pub const LOGGER_NAME: &str = "Crash Logger";

/// DLLs that appear in nearly every crash without being the cause of it.
const SYSTEM_MODULES: &[&str] = &[
    "skyrimse.exe", "ntdll.dll", "kernel32.dll", "kernelbase.dll", "user32.dll", "win32u.dll", "gdi32.dll",
    "gdi32full.dll", "ucrtbase.dll", "msvcrt.dll", "msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll",
    "combase.dll", "rpcrt4.dll", "ole32.dll", "oleaut32.dll", "sechost.dll", "advapi32.dll", "shell32.dll",
    "d3d11.dll", "d3d12.dll", "dxgi.dll", "d3dcompiler_47.dll", "dinput8.dll", "dsound.dll", "xinput1_3.dll",
    "xinput9_1_0.dll", "xaudio2_7.dll", "x3daudio1_7.dll", "binkw64.dll", "steam_api64.dll",
    "gameoverlayrenderer64.dll", "crashlogger.dll", "trainwreck.dll", "nvwgf2umx.dll", "nvapi64.dll",
    "nvngx.dll", "amdxc64.dll", "atidxx64.dll", "igd10iumd64.dll", "dbghelp.dll", "wininet.dll", "ws2_32.dll",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub path: PathBuf,
    /// Seconds since the Unix epoch when the report was written.
    pub time: u64,
    /// The exception, in the log's own words ("EXCEPTION_ACCESS_VIOLATION").
    pub error: String,
    /// Where the crash happened, as the log names it ("SkyrimSE.exe+0BDD1B0").
    pub location: String,
    /// Files named near the crash, most likely first: SKSE DLLs from the call stack,
    /// then plugins and other files the log flagged as relevant. Game and system files are left out.
    pub culprits: Vec<String>,
}

/// Where SKSE (and so Crash Logger) keeps its logs.
pub fn logs_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let docs = directories::UserDirs::new()?.document_dir()?.to_path_buf();
    // The game runs inside a Proton prefix here, so its "Documents" is the one in there.
    #[cfg(not(windows))]
    let docs = crate::game::wine_user_dir()?.join("Documents");
    Some(docs.join("My Games").join("Skyrim Special Edition").join("SKSE"))
}

/// Whether a report writer is in place, so crashes will actually leave something behind.
pub fn logger_installed(data_dir: &Path) -> bool {
    let plugins = data_dir.join("SKSE").join("Plugins");
    plugins.join("CrashLogger.dll").is_file() || plugins.join("Trainwreck.dll").is_file()
}

/// Every crash report found, newest first.
pub fn find_reports() -> Vec<Report> {
    let Some(dir) = logs_dir() else { return Vec::new() };
    let mut files: Vec<PathBuf> = Vec::new();
    for d in [dir.clone(), dir.join("Crashlogs")] {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if name.starts_with("crash-") && name.ends_with(".log") {
                files.push(path);
            }
        }
    }
    let mut reports: Vec<Report> = files.into_iter().filter_map(|p| load(&p)).collect();
    reports.sort_by(|a, b| b.time.cmp(&a.time));
    reports
}

fn load(path: &Path) -> Option<Report> {
    let text = fs::read_to_string(path).ok()?;
    let time = fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(parse(path.to_path_buf(), time, &text))
}

/// Pulls the headline facts out of a Crash Logger / Trainwreck report.
pub fn parse(path: PathBuf, time: u64, text: &str) -> Report {
    let mut error = String::new();
    let mut location = String::new();
    let mut dlls: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut in_stack = false;

    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("unhandled exception") {
            // Unhandled exception "EXCEPTION_ACCESS_VIOLATION" at 0x7FF6... SkyrimSE.exe+0BDD1B0
            if let Some(q) = t.split('"').nth(1) {
                error = q.to_string();
            }
            location = t
                .split_whitespace()
                .find(|w| w.contains('+') && (lower_has_ext(w, ".dll+") || lower_has_ext(w, ".exe+")))
                .unwrap_or_default()
                .to_string();
            continue;
        }
        if lower.starts_with("probable call stack") || lower.starts_with("probable callstack") {
            in_stack = true;
            continue;
        }
        if in_stack && t.ends_with(':') && !t.starts_with('[') {
            in_stack = false;
        }
        if in_stack {
            for w in t.split_whitespace() {
                if let Some(m) = module_name(w) {
                    push_unique(&mut dlls, m);
                }
            }
        }
        // Registers, stack and "possible relevant objects" mention forms by the file they came
        // from, e.g. `File: "SomeMod.esp"` — those are the plugins involved in the crash.
        let mut rest = t;
        while let Some(i) = rest.find("File: \"") {
            let after = &rest[i + 7..];
            if let Some(end) = after.find('"') {
                let name = &after[..end];
                if name.ends_with(".esp") || name.ends_with(".esm") || name.ends_with(".esl") {
                    push_unique(&mut files, name.to_string());
                }
                rest = &after[end..];
            } else {
                break;
            }
        }
    }

    let mut culprits: Vec<String> = dlls.into_iter().filter(|d| !is_system(d)).collect();
    for f in files {
        if !is_core_plugin(&f) {
            push_unique(&mut culprits, f);
        }
    }
    Report { path, time, error, location, culprits }
}

fn lower_has_ext(word: &str, ext: &str) -> bool {
    word.to_ascii_lowercase().contains(ext)
}

/// "CommunityShaders.dll+1234" → "CommunityShaders.dll".
fn module_name(word: &str) -> Option<String> {
    let (name, _) = word.split_once('+')?;
    let lower = name.to_ascii_lowercase();
    (lower.ends_with(".dll") || lower.ends_with(".exe")).then(|| name.to_string())
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !v.iter().any(|x| x.eq_ignore_ascii_case(&s)) {
        v.push(s);
    }
}

fn is_system(module: &str) -> bool {
    let m = module.to_ascii_lowercase();
    SYSTEM_MODULES.contains(&m.as_str()) || m.starts_with("skse64_") || m.starts_with("api-ms-") || m.starts_with("nv")
}

fn is_core_plugin(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "skyrim.esm" | "update.esm" | "dawnguard.esm" | "hearthfires.esm" | "dragonborn.esm" | "_resourcepack.esl"
    )
}

/// "just now", "4 minutes ago", "yesterday" — for a report timestamp.
pub fn ago(time: u64) -> String {
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs();
    let s = now.saturating_sub(time);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => plural(s / 60, "minute"),
        3600..=86399 => plural(s / 3600, "hour"),
        86400..=172799 => "yesterday".into(),
        _ => plural(s / 86400, "day"),
    }
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 { format!("1 {unit} ago") } else { format!("{n} {unit}s ago") }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"Skyrim SSE v1.6.1170
CrashLoggerSSE v1-25-0-0 Aug 21 2026 09:21:03

Unhandled exception "EXCEPTION_ACCESS_VIOLATION" at 0x7FF6E4E4D1B0 SkyrimSE.exe+0BDD1B0	mov rax, [rcx+0x08]

SYSTEM SPECS:
	OS: Microsoft Windows 11 Home v10.0.26200
	GPU #1: Nvidia GA104 [GeForce RTX 3070]

PROBABLE CALL STACK:
	[0] 0x7FF6E4E4D1B0 SkyrimSE.exe+0BDD1B0 -> 12345+0x10
	[1] 0x7FFC12345678 CommunityShaders.dll+0012345 -> ...
	[2] 0x7FFC22222222 KERNELBASE.dll+0001111
	[3] 0x7FFC33333333 EngineFixes.dll+0000ABC
	[4] 0x7FFC44444444 ntdll.dll+0000123

REGISTERS:
	RAX 0x0 (size_t) [0]
	RCX 0x2A1B3C4D (TESObjectREFR*)
		File: "Vyrthland Landscapes.esp"
		Flags: 0x00000008
		FormID: 0x0A001234

STACK:
	[RSP+0 ] 0x1 (size_t)
	[RSP+8 ] 0x2 (TESForm*)
		File: "Skyrim.esm"

MODULES:
	SkyrimSE.exe 0x7FF6E4270000
	CommunityShaders.dll 0x7FFC12000000
	SomethingElse.dll 0x7FFC99000000

PLUGINS:
	[00] Skyrim.esm
	[01] Vyrthland Landscapes.esp
"#;

    #[test]
    fn parses_crash_logger_report() {
        let r = parse(PathBuf::from("crash.log"), 0, SAMPLE);
        assert_eq!(r.error, "EXCEPTION_ACCESS_VIOLATION");
        assert_eq!(r.location, "SkyrimSE.exe+0BDD1B0");
        // DLLs from the stack first (system ones dropped), then plugins the log flagged;
        // nothing from the MODULES list, which is just everything loaded.
        assert_eq!(r.culprits, vec!["CommunityShaders.dll", "EngineFixes.dll", "Vyrthland Landscapes.esp"]);
    }

    #[test]
    fn empty_report_has_no_culprits() {
        let r = parse(PathBuf::from("crash.log"), 0, "garbage\n");
        assert!(r.error.is_empty() && r.culprits.is_empty());
    }
}
