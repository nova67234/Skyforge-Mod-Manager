//! Nexus Mods: account check, mod search, file lists, downloads and `nxm://` links.
//!
//! Everything here is blocking and meant to be called from a worker thread.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

pub const GAME: &str = "skyrimspecialedition";
/// Nexus's numeric id for Skyrim Special Edition, used by the GraphQL API.
const GAME_ID: u64 = 1704;
/// Nexus mod id of the SKSE placeholder page — the real download lives on skse.silverlock.org.
pub const SKSE_MOD_ID: u64 = 30379;
pub const SITE_URL: &str = "https://www.nexusmods.com/skyrimspecialedition/mods/";
pub const API_KEY_URL: &str = "https://www.nexusmods.com/settings/api-keys";
const API: &str = "https://api.nexusmods.com/v1";
const GRAPHQL: &str = "https://api.nexusmods.com/v2/graphql";
const USER_AGENT: &str = concat!("SkyrimModManager/", env!("CARGO_PKG_VERSION"), " (Windows)");

/// Who the API key belongs to.
#[derive(Clone, Debug, Deserialize)]
pub struct User {
    pub name: String,
    #[serde(default)]
    pub is_premium: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub picture_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FileInfo {
    pub file_id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub category_name: Option<String>,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub size_kb: u64,
}

impl FileInfo {
    pub fn category(&self) -> &str {
        self.category_name.as_deref().unwrap_or("")
    }
    pub fn is_main(&self) -> bool {
        self.category() == "MAIN"
    }
    /// Files worth offering: the current main, update and optional ones.
    pub fn is_current(&self) -> bool {
        matches!(self.category(), "MAIN" | "UPDATE" | "OPTIONAL" | "MISCELLANEOUS")
    }
}

/// One search hit from the GraphQL API.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMod {
    pub mod_id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub endorsements: u64,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SearchPage {
    pub total: u64,
    pub mods: Vec<SearchMod>,
}

/// One thing a mod's page says it needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requirement {
    /// Another Nexus mod, or 0 when it's something off-site (a redistributable, a tool).
    pub mod_id: u64,
    pub name: String,
    /// The author's note about it ("only for the optional file"), often empty.
    pub notes: String,
    /// Where to get an off-site requirement.
    pub url: String,
}

/// What a mod's Nexus page lists under Requirements.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Requirements {
    pub mods: Vec<Requirement>,
    /// Official add-ons it needs ("Fishing", "Survival Mode") — the Anniversary content.
    pub dlc: Vec<String>,
}

impl Requirements {
    pub fn is_empty(&self) -> bool {
        self.mods.is_empty() && self.dlc.is_empty()
    }
}

/// Download progress shared between the worker and the UI.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| self.done.load(Ordering::Relaxed) as f32 / total as f32)
    }
}

/// A `nxm://skyrimspecialedition/mods/123/files/456?key=..&expires=..&user_id=..` link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NxmLink {
    pub game: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub key: Option<String>,
    pub expires: Option<u64>,
}

impl NxmLink {
    pub fn parse(url: &str) -> Option<Self> {
        let rest = url.strip_prefix("nxm://")?;
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
        let [game, "mods", mod_id, "files", file_id] = parts[..] else {
            return None;
        };
        let mut key = None;
        let mut expires = None;
        for pair in query.split('&') {
            match pair.split_once('=') {
                Some(("key", v)) if !v.is_empty() => key = Some(v.to_string()),
                Some(("expires", v)) => expires = v.parse().ok(),
                _ => {}
            }
        }
        Some(Self {
            game: game.to_ascii_lowercase(),
            mod_id: mod_id.parse().ok()?,
            file_id: file_id.parse().ok()?,
            key,
            expires,
        })
    }
}

pub struct Client {
    http: reqwest::blocking::Client,
    key: String,
}

impl Client {
    pub fn new(key: &str) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { http, key: key.trim().to_string() })
    }

    fn get(&self, path: &str) -> reqwest::blocking::RequestBuilder {
        self.http.get(format!("{API}/{path}")).header("apikey", &self.key)
    }

    fn json<T: serde::de::DeserializeOwned>(resp: reqwest::blocking::Response) -> Result<T> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp.json()?);
        }
        let body = resp.text().unwrap_or_default();
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
            .unwrap_or_default();
        match status.as_u16() {
            401 => bail!("That API key isn't valid. Copy it again from your Nexus account page."),
            403 if message.contains("premium") || message.contains("Premium") => {
                bail!("Direct downloads need Nexus Premium. Use the “Mod Manager Download” button on the website instead.")
            }
            404 => bail!("Nexus Mods couldn't find that mod or file."),
            429 => bail!("Nexus Mods is rate limiting requests. Try again in a little while."),
            _ if !message.is_empty() => bail!("Nexus Mods: {message}"),
            _ => bail!("Nexus Mods returned {status}"),
        }
    }

    /// Checks the key and returns the account it belongs to.
    pub fn validate(&self) -> Result<User> {
        let resp = self.get("users/validate.json").send().context("Couldn't reach Nexus Mods")?;
        Self::json(resp)
    }

    pub fn mod_info(&self, mod_id: u64) -> Result<ModInfo> {
        let resp = self
            .get(&format!("games/{GAME}/mods/{mod_id}.json"))
            .send()
            .context("Couldn't reach Nexus Mods")?;
        Self::json(resp)
    }

    pub fn files(&self, mod_id: u64) -> Result<Vec<FileInfo>> {
        #[derive(Deserialize)]
        struct Files {
            files: Vec<FileInfo>,
        }
        let resp = self
            .get(&format!("games/{GAME}/mods/{mod_id}/files.json"))
            .send()
            .context("Couldn't reach Nexus Mods")?;
        let files: Files = Self::json(resp)?;
        Ok(files.files)
    }

    /// Resolves a CDN URL for a file. Premium accounts need no `key`/`expires`; free
    /// accounts must pass the ones from an `nxm://` link.
    pub fn download_link(&self, mod_id: u64, file_id: u64, key: Option<&str>, expires: Option<u64>) -> Result<String> {
        #[derive(Deserialize)]
        struct Link {
            #[serde(rename = "URI")]
            uri: String,
        }
        let mut req = self.get(&format!("games/{GAME}/mods/{mod_id}/files/{file_id}/download_link.json"));
        if let (Some(k), Some(e)) = (key, expires) {
            req = req.query(&[("key", k.to_string()), ("expires", e.to_string())]);
        }
        let resp = req.send().context("Couldn't reach Nexus Mods")?;
        let links: Vec<Link> = Self::json(resp)?;
        links.into_iter().next().map(|l| l.uri).context("Nexus Mods didn't return a download link")
    }

    /// Streams `url` into `dest`, reporting progress. Writes to a `.part` file first so a
    /// half-finished download never looks like a real archive.
    pub fn download(&self, url: &str, dest: &Path, progress: &Arc<Progress>) -> Result<()> {
        let mut resp = self.http.get(url).timeout(Duration::from_secs(3600)).send().context("Download failed to start")?;
        if !resp.status().is_success() {
            bail!("Download failed ({})", resp.status());
        }
        progress.total.store(resp.content_length().unwrap_or(0), Ordering::Relaxed);
        let part = dest.with_extension(format!(
            "{}.part",
            dest.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()
        ));
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(&part)?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if progress.cancel.load(Ordering::Relaxed) {
                drop(file);
                let _ = std::fs::remove_file(&part);
                bail!("Download cancelled");
            }
            let n = resp.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            progress.done.fetch_add(n as u64, Ordering::Relaxed);
        }
        file.flush()?;
        drop(file);
        let _ = std::fs::remove_file(dest);
        std::fs::rename(&part, dest)?;
        Ok(())
    }

    /// Fetches an image (thumbnail) as raw bytes.
    pub fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self.http.get(url).send()?;
        if !resp.status().is_success() {
            bail!("{}", resp.status());
        }
        Ok(resp.bytes()?.to_vec())
    }

    /// Searches Skyrim SE mods by name; an empty query lists the most endorsed mods.
    pub fn search(&self, query: &str, offset: u64, count: u64) -> Result<SearchPage> {
        const QUERY: &str = "query Mods($filter: ModsFilter, $count: Int, $offset: Int, $sort: [ModsSort!]) { \
            mods(filter: $filter, count: $count, offset: $offset, sort: $sort) { totalCount \
            nodes { modId name summary version author downloads endorsements thumbnailUrl } } }";
        let mut filter = serde_json::json!({
            "gameDomainName": { "value": GAME, "op": "EQUALS" },
            "adultContent": { "value": false, "op": "EQUALS" },
        });
        let q = query.trim();
        if !q.is_empty() {
            filter["name"] = serde_json::json!({ "value": q, "op": "WILDCARD" });
        }
        let sort = if q.is_empty() {
            serde_json::json!({ "endorsements": { "direction": "DESC" } })
        } else {
            serde_json::json!({ "downloads": { "direction": "DESC" } })
        };
        let body = serde_json::json!({
            "query": QUERY,
            "variables": { "filter": filter, "count": count, "offset": offset, "sort": sort },
        });
        // Search is public; the key is only needed for file lists and downloads.
        let resp = self
            .http
            .post(GRAPHQL)
            .json(&body)
            .send()
            .context("Couldn't reach Nexus Mods")?;
        if !resp.status().is_success() {
            bail!("Nexus Mods search failed ({})", resp.status());
        }
        #[derive(Deserialize)]
        struct Resp {
            data: Option<Data>,
            #[serde(default)]
            errors: Vec<serde_json::Value>,
        }
        #[derive(Deserialize)]
        struct Data {
            mods: Mods,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Mods {
            total_count: u64,
            nodes: Vec<SearchMod>,
        }
        let r: Resp = resp.json()?;
        match r.data {
            Some(d) => {
                let mut mods = d.mods.nodes;
                for m in &mut mods {
                    m.summary = strip_bbcode(&m.summary);
                }
                Ok(SearchPage { total: d.mods.total_count, mods })
            }
            None => {
                let msg = r.errors.first().and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("unknown error");
                bail!("Nexus Mods search failed: {msg}")
            }
        }
    }

    /// The Requirements section of each mod's page, in one request. Mods Nexus doesn't
    /// know are simply absent from the result.
    pub fn requirements(&self, mod_ids: &[u64]) -> Result<std::collections::HashMap<u64, Requirements>> {
        const QUERY: &str = "query Reqs($ids: [CompositeIdInput!]!) { legacyMods(ids: $ids) { nodes { modId \
            modRequirements { nexusRequirements { nodes { modId modName notes url externalRequirement } } \
            dlcRequirements { notes gameExpansion { name } } } } } }";
        let ids: Vec<serde_json::Value> = mod_ids.iter().map(|id| serde_json::json!({ "gameId": GAME_ID, "modId": id })).collect();
        let body = serde_json::json!({ "query": QUERY, "variables": { "ids": ids } });
        let resp = self
            .http
            .post(GRAPHQL)
            .json(&body)
            .send()
            .context("Couldn't reach Nexus Mods")?;
        if !resp.status().is_success() {
            bail!("Nexus Mods returned {}", resp.status());
        }
        #[derive(Deserialize)]
        struct Resp {
            data: Option<Data>,
            #[serde(default)]
            errors: Vec<serde_json::Value>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Data {
            legacy_mods: Nodes<ModNode>,
        }
        #[derive(Deserialize)]
        struct Nodes<T> {
            nodes: Vec<T>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ModNode {
            mod_id: u64,
            mod_requirements: ModReqs,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ModReqs {
            nexus_requirements: Nodes<ReqNode>,
            #[serde(default)]
            dlc_requirements: Vec<DlcNode>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ReqNode {
            // Ids come back as strings here.
            #[serde(default)]
            mod_id: String,
            #[serde(default)]
            mod_name: String,
            #[serde(default)]
            notes: Option<String>,
            #[serde(default)]
            url: String,
            #[serde(default)]
            external_requirement: bool,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct DlcNode {
            game_expansion: Option<Named>,
        }
        #[derive(Deserialize)]
        struct Named {
            name: String,
        }
        let r: Resp = resp.json()?;
        let Some(d) = r.data else {
            let msg = r.errors.first().and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("unknown error");
            bail!("Nexus Mods: {msg}")
        };
        let mut out = std::collections::HashMap::new();
        for node in d.legacy_mods.nodes {
            let mods = node
                .mod_requirements
                .nexus_requirements
                .nodes
                .into_iter()
                .map(|n| Requirement {
                    mod_id: if n.external_requirement { 0 } else { n.mod_id.parse().unwrap_or(0) },
                    name: unescape_html(n.mod_name.trim()),
                    notes: unescape_html(strip_bbcode(n.notes.as_deref().unwrap_or("")).trim()),
                    url: n.url,
                })
                .filter(|r| !r.name.is_empty())
                .collect();
            let dlc = node
                .mod_requirements
                .dlc_requirements
                .into_iter()
                .filter_map(|d| d.game_expansion.map(|g| unescape_html(g.name.trim())))
                .filter(|s| !s.is_empty())
                .collect();
            out.insert(node.mod_id, Requirements { mods, dlc });
        }
        Ok(out)
    }
}

/// The API hands names back HTML-escaped ("Saints &amp; Seducers").
fn unescape_html(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
}

// ---------------------------------------------------------------------------
// nxm:// protocol registration (per-user, no admin rights needed)
// ---------------------------------------------------------------------------

const PROTOCOL_KEY: &str = r"Software\Classes\nxm";

fn open_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// True when "Mod Manager Download" links already open this executable.
pub fn protocol_registered(exe: &Path) -> bool {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    hkcu.open_subkey(format!(r"{PROTOCOL_KEY}\shell\open\command"))
        .and_then(|k| k.get_value::<String, _>(""))
        .map(|v| v.eq_ignore_ascii_case(&open_command(exe)))
        .unwrap_or(false)
}

pub fn register_protocol(exe: &Path) -> Result<()> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (root, _) = hkcu.create_subkey(PROTOCOL_KEY).context("Couldn't write to the registry")?;
    root.set_value("", &"URL:Nexus Mods Protocol")?;
    root.set_value("URL Protocol", &"")?;
    let (icon, _) = root.create_subkey("DefaultIcon")?;
    icon.set_value("", &format!("\"{}\",0", exe.display()))?;
    let (cmd, _) = root.create_subkey(r"shell\open\command")?;
    cmd.set_value("", &open_command(exe))?;
    Ok(())
}

pub fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.canonicalize().ok().or(Some(p))).map(|p| {
        // canonicalize() adds a \\?\ prefix that Explorer and the registry don't like.
        let s = p.to_string_lossy();
        PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s))
    })
}

/// Mod summaries can carry forum markup like `[url=…]text[/url]`; keep just the text.
fn strip_bbcode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "25.5M", "63K", "812".
pub fn compact_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => {
            let k = n as f64 / 1000.0;
            if k < 10.0 { format!("{k:.1}K") } else { format!("{k:.0}K") }
        }
        _ => {
            let m = n as f64 / 1_000_000.0;
            if m < 10.0 { format!("{m:.1}M") } else { format!("{m:.0}M") }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nxm_links() {
        let l = NxmLink::parse("nxm://skyrimspecialedition/mods/12604/files/234567?key=abc&expires=1700000000&user_id=42").unwrap();
        assert_eq!(l.game, "skyrimspecialedition");
        assert_eq!(l.mod_id, 12604);
        assert_eq!(l.file_id, 234567);
        assert_eq!(l.key.as_deref(), Some("abc"));
        assert_eq!(l.expires, Some(1700000000));
        let plain = NxmLink::parse("nxm://SkyrimSpecialEdition/mods/1/files/2").unwrap();
        assert_eq!(plain.game, "skyrimspecialedition");
        assert!(plain.key.is_none());
        assert!(NxmLink::parse("nxm://skyrimspecialedition/collections/abc").is_none());
        assert!(NxmLink::parse("https://nexusmods.com").is_none());
    }

    /// Hits the live API; run with `cargo test -- --ignored live_search`.
    #[test]
    #[ignore]
    fn live_search() {
        let c = Client::new("").unwrap();
        let page = c.search("", 0, 5).unwrap();
        assert!(page.total > 0, "no total");
        assert!(!page.mods.is_empty(), "no mods");
        let page = c.search("skse", 0, 5).unwrap();
        assert!(page.mods.iter().any(|m| m.name.to_ascii_lowercase().contains("skse")));
    }

    #[test]
    fn strips_bbcode() {
        assert_eq!(strip_bbcode("comes with [url=https://x.y/?a]SSE Engine Fixes[/url] (skse64)"), "comes with SSE Engine Fixes (skse64)");
        assert_eq!(strip_bbcode("plain  text"), "plain text");
        assert_eq!(strip_bbcode("unbalanced ] bracket"), "unbalanced ] bracket");
    }

    #[test]
    fn compact_counts() {
        assert_eq!(compact_count(812), "812");
        assert_eq!(compact_count(63_441), "63K");
        assert_eq!(compact_count(2_477_307), "2.5M");
        assert_eq!(compact_count(25_534_102), "26M");
    }
}
