//! AppImage source: a catalogue from AppImageHub, verified downloads from GitHub releases, and
//! per-user installs.
//!
//! An AppImage is a single executable file. Unlike APT, Flatpak or Snap it has no package manager,
//! no sandbox and no signature the store can check, so this source is deliberately narrow:
//!
//! * The catalogue is the public AppImageHub feed. Only projects hosted on GitHub are offered.
//! * An install downloads the AppImage of the project's **latest stable GitHub release** for this
//!   machine's architecture, over https, from `github.com` only. The download is checked against the
//!   SHA-256 digest GitHub publishes for the file (when there is one), its size, and the AppImage
//!   file header, before it is made executable.
//! * Everything stays inside the user's home: `~/.local/share/nexa-store/appimages/<owner>/<repo>/`
//!   plus one launcher in `~/.local/share/applications/`. No administrator rights are involved.
//! * The confirmation says plainly that an AppImage runs with all the permissions of the user.
//!
//! Nothing here starts a shell. Identifiers are validated, paths are built from validated parts,
//! and every address is checked against an allow-list, including each redirect.
use crate::backend::{Package, PackageDetails, Source};
use crate::categories;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    os::unix::{fs::{OpenOptionsExt, PermissionsExt}, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, String>;

const FEED_URL: &str = "https://appimage.github.io/feed.json";
const ICON_BASE: &str = "https://appimage.github.io/database/";
const FEED_LIMIT: u64 = 16 * 1024 * 1024;
const FEED_TTL: Duration = Duration::from_secs(3 * 24 * 3600);
const FEED_RETRY: Duration = Duration::from_secs(120);
const API_LIMIT: u64 = 4 * 1024 * 1024;
const ICON_LIMIT: u64 = 512 * 1024;
const APP_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const MANIFEST_LIMIT: u64 = 64 * 1024;
const RELEASE_TTL: Duration = Duration::from_secs(600);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const SEARCH_LIMIT: usize = 60;
const MARKER: &str = "X-Nexa-Store-AppImage";
const USER_AGENT: &str = concat!("nexa-store/", env!("CARGO_PKG_VERSION"), " (+https://github.com/Foglio24/nexa-store)");

// ---- Addresses and network -----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose { Catalog, Api, Download }

fn host_allowed(host: &str, purpose: Purpose) -> bool {
    let host = host.to_ascii_lowercase();
    match purpose {
        Purpose::Catalog => host == "appimage.github.io",
        Purpose::Api => host == "api.github.com",
        Purpose::Download => host == "github.com" || (host.ends_with(".githubusercontent.com") && host.len() > ".githubusercontent.com".len()),
    }
}

/// Accept only `https://<allowed host>/…` without credentials, ports or odd characters. Returns the path.
fn checked_url(url: &str, purpose: Purpose) -> Result<String> {
    let rest = url.strip_prefix("https://").ok_or_else(|| tr!("Only https addresses are allowed."))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let valid = !authority.is_empty() && !authority.contains(['@', ':', '\\']) && url.len() <= 4096
        && !url.chars().any(|c| c.is_control() || c == ' ') && host_allowed(authority, purpose);
    if !valid { return Err(tr!("Address not allowed: {}", authority)); }
    Ok(rest[authority.len()..].to_string())
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .timeout_write(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build())
}

type Stream = Box<dyn Read + Send + Sync>;
/// How the module reads an address; tests replace it so nothing touches the network.
type Fetch<'a> = &'a dyn Fn(&str, Purpose) -> Result<(Stream, Option<u64>)>;

/// Open an address, following redirects by hand so that every hop is checked against the allow-list.
fn open_url(url: &str, purpose: Purpose) -> Result<(Stream, Option<u64>)> {
    let mut current = url.to_string();
    for _ in 0..6 {
        checked_url(&current, purpose)?;
        let request = agent().get(&current).set("Accept", if purpose == Purpose::Api { "application/vnd.github+json" } else { "*/*" })
            .set("X-GitHub-Api-Version", "2022-11-28");
        match request.call() {
            Ok(response) if (300..400).contains(&response.status()) => {
                let location = response.header("Location").ok_or_else(|| tr!("The server sent a redirect without a destination."))?.to_string();
                current = if location.starts_with('/') {
                    let host = current.strip_prefix("https://").and_then(|rest| rest.split('/').next()).unwrap_or("");
                    format!("https://{host}{location}")
                } else { location };
            },
            Ok(response) => {
                let length = response.header("Content-Length").and_then(|value| value.parse().ok());
                return Ok((Box::new(response.into_reader()), length));
            },
            Err(ureq::Error::Status(code, response)) => {
                let exhausted = response.header("x-ratelimit-remaining") == Some("0");
                return Err(if matches!(code, 403 | 429) && exhausted { tr!("GitHub's request limit has been reached. Try again in a while.") }
                    else if code == 404 { tr!("Not found on GitHub (error 404).") }
                    else { tr!("The server answered with error {}.", code) });
            },
            Err(ureq::Error::Transport(error)) => return Err(tr!("Network error: {}", error)),
        }
    }
    Err(tr!("Too many redirects."))
}

fn read_limited(mut stream: Stream, limit: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    (&mut stream).take(limit + 1).read_to_end(&mut data).map_err(|e| tr!("Network error: {}", e))?;
    if data.len() as u64 > limit { return Err(tr!("The answer is larger than the safety limit.")); }
    Ok(data)
}

fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }

// ---- Identifiers, files and paths -----------------------------------------------------------

/// `owner/repo` exactly as GitHub allows it: letters, digits, `-`, `_` and `.`, not starting with `.` or `-`.
pub fn valid_id(id: &str) -> bool {
    let part = |text: &str, max: usize| !text.is_empty() && text.len() <= max && !text.starts_with(['.', '-'])
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    id.split_once('/').map(|(owner, repo)| part(owner, 39) && part(repo, 100)).unwrap_or(false)
}

fn id_key(id: &str) -> String { id.to_ascii_lowercase() }

fn safe_file_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 200 && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-~".contains(&b)) && name.to_ascii_lowercase().ends_with(".appimage")
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || "._+-~".contains(c) { c } else { '_' }).collect();
    cleaned.trim_start_matches('.').to_string()
}

fn clean_text(text: &str, limit: usize) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().filter(|c| !c.is_control()).take(limit).collect()
}

/// The folders the module works in. Tests use temporary ones.
struct Paths { base: PathBuf, launchers: PathBuf }

impl Paths {
    fn system() -> Result<Self> {
        let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from).filter(|path| path.is_absolute()).map(|home| home.join(".local/share")))
            .ok_or_else(|| tr!("The home folder could not be found."))?;
        Ok(Self { base: data.join("nexa-store/appimages"), launchers: data.join("applications") })
    }
    fn app_dir(&self, id: &str) -> PathBuf {
        let key = id_key(id);
        let (owner, repo) = key.split_once('/').unwrap_or(("_", "_"));
        self.base.join(owner).join(repo)
    }
}

fn launcher_name(id: &str) -> String {
    let digest = Sha256::digest(id_key(id).as_bytes());
    format!("nexa-appimage-{}.desktop", &hex(&digest)[..12])
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct Manifest {
    id: String,
    name: String,
    version: String,
    file: String,
    sha256: String,
    url: String,
    summary: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    desktop: String,
    installed_at: u64,
}

fn regular_file(path: &Path) -> bool { fs::symlink_metadata(path).map(|meta| meta.is_file()).unwrap_or(false) }
fn real_dir(path: &Path) -> bool { fs::symlink_metadata(path).map(|meta| meta.is_dir()).unwrap_or(false) }

/// The record of an installed AppImage, only if it is intact and inside the store folder. A damaged or
/// hand-edited record is ignored rather than trusted.
fn read_manifest(paths: &Paths, id: &str) -> Option<Manifest> {
    if !valid_id(id) { return None; }
    let dir = paths.app_dir(id);
    if !real_dir(&dir) { return None; }
    let file = dir.join("meta.json");
    if !regular_file(&file) { return None; }
    let mut text = String::new();
    fs::File::open(&file).ok()?.take(MANIFEST_LIMIT + 1).read_to_string(&mut text).ok()?;
    if text.len() as u64 > MANIFEST_LIMIT { return None; }
    let manifest: Manifest = serde_json::from_str(&text).ok()?;
    let intact = valid_id(&manifest.id) && id_key(&manifest.id) == id_key(id) && safe_file_name(&manifest.file)
        && manifest.desktop == launcher_name(id) && regular_file(&dir.join(&manifest.file))
        && manifest.icon.as_deref().map(|icon| icon == "icon.png").unwrap_or(true);
    intact.then_some(manifest)
}

fn list_manifests(paths: &Paths) -> Vec<Manifest> {
    let mut result = Vec::new();
    let Ok(owners) = fs::read_dir(&paths.base) else { return result };
    for owner in owners.flatten() {
        if !real_dir(&owner.path()) { continue; }
        let Ok(repos) = fs::read_dir(owner.path()) else { continue };
        for repo in repos.flatten() {
            let id = format!("{}/{}", owner.file_name().to_string_lossy(), repo.file_name().to_string_lossy());
            if let Some(manifest) = read_manifest(paths, &id) { result.push(manifest); }
        }
    }
    result.sort_by_key(|manifest| manifest.name.to_lowercase());
    result
}

/// Ids (lower case) of the AppImages installed through the store.
pub fn installed_ids() -> HashSet<String> {
    Paths::system().map(|paths| list_manifests(&paths).into_iter().map(|manifest| id_key(&manifest.id)).collect()).unwrap_or_default()
}

// ---- Catalogue ------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub summary: String,
    pub categories: Vec<&'static str>,
    /// Freedesktop category names that the launcher declares.
    pub freedesktop: Vec<String>,
    pub license: Option<String>,
    pub author: Option<String>,
    pub icon: Option<String>,
    pub name_key: String,
}

fn icon_address(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').collect();
    let valid = !path.is_empty() && path.len() <= 300 && segments.iter().all(|s| !s.is_empty() && *s != "." && *s != ".." && !s.chars().any(char::is_control))
        && path.to_ascii_lowercase().ends_with(".png");
    valid.then(|| format!("{ICON_BASE}{}", segments.iter().map(|segment| crate::backend::uri_component(segment)).collect::<Vec<_>>().join("/")))
}

/// A JSON value as text: strings as they are, numbers written out (the feed has a few), anything else nothing.
fn text_of(value: &serde_json::Value) -> Option<String> {
    match value { serde_json::Value::String(text) => Some(text.clone()), serde_json::Value::Number(number) => Some(number.to_string()), _ => None }
}

fn texts_of(value: &serde_json::Value) -> Vec<String> {
    value.as_array().map(|items| items.iter().filter_map(text_of).collect()).unwrap_or_default()
}

/// One feed item. The feed is hand-maintained by many people, so each item is read on its own and a
/// malformed one is skipped instead of making the whole catalogue unreadable.
fn entry_from(item: &serde_json::Value) -> Option<Entry> {
    let id = item["links"].as_array()?.iter().find(|link| link["type"].as_str() == Some("GitHub")).and_then(|link| text_of(&link["url"]))?.trim().to_string();
    let name = clean_text(&text_of(&item["name"])?.replace('_', " "), 80);
    if !valid_id(&id) || name.is_empty() { return None; }
    let description = clean_text(&text_of(&item["description"]).unwrap_or_default(), 600);
    let summary = if description.chars().count() > 140 { format!("{}…", description.chars().take(139).collect::<String>().trim_end()) } else { description.clone() };
    let raw = texts_of(&item["categories"]);
    let freedesktop: Vec<String> = raw.iter().filter(|name| categories::from_freedesktop(name).is_some()).cloned().collect();
    Some(Entry {
        name_key: categories::name_key(&name),
        categories: categories::from_freedesktop_list(&raw),
        license: text_of(&item["license"]).map(|text| clean_text(&text, 60)).filter(|text| !text.is_empty()),
        author: item["authors"].as_array().and_then(|authors| authors.iter().find_map(|author| text_of(&author["name"]))).map(|text| clean_text(&text, 60)).filter(|text| !text.is_empty()),
        icon: texts_of(&item["icons"]).iter().find_map(|path| icon_address(path)),
        id, name, description, summary, freedesktop,
    })
}

fn parse_feed(data: &[u8]) -> Result<Vec<Entry>> {
    let root: serde_json::Value = serde_json::from_slice(data).map_err(|e| tr!("The AppImage catalog is not valid: {}", e))?;
    let items = root["items"].as_array().ok_or_else(|| tr!("The AppImage catalog is not valid: {}", "items"))?;
    let mut seen = HashSet::new();
    let mut entries: Vec<Entry> = items.iter().filter_map(entry_from).filter(|entry| seen.insert(id_key(&entry.id))).collect();
    entries.sort_by(|a, b| a.name_key.cmp(&b.name_key).then_with(|| a.id.cmp(&b.id)));
    Ok(entries)
}

struct Feed { loaded: Instant, ttl: Duration, entries: Arc<Vec<Entry>> }
static FEED: Mutex<Option<Feed>> = Mutex::new(None);

fn cache_file() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from).filter(|path| path.is_absolute()).map(|home| home.join(".cache")))?;
    Some(cache.join("nexa-store/appimage-feed.json"))
}

fn write_atomically(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    let mut random = [0u8; 6];
    let _ = getrandom::getrandom(&mut random);
    let temporary = path.with_extension(format!("tmp-{}", hex(&random)));
    let result = (|| {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}

fn load_feed() -> (Arc<Vec<Entry>>, Duration) {
    let cache = cache_file();
    let age = cache.as_ref().and_then(|path| fs::metadata(path).ok()).and_then(|meta| meta.modified().ok()).and_then(|time| SystemTime::now().duration_since(time).ok());
    let read_cache = || cache.as_ref().and_then(|path| fs::File::open(path).ok()).and_then(|file| {
        let mut data = Vec::new();
        file.take(FEED_LIMIT + 1).read_to_end(&mut data).ok()?;
        (data.len() as u64 <= FEED_LIMIT).then_some(data)
    }).and_then(|data| parse_feed(&data).ok());
    if age.is_some_and(|age| age < FEED_TTL) {
        if let Some(entries) = read_cache() { return (Arc::new(entries), FEED_TTL); }
    }
    let downloaded = open_url(FEED_URL, Purpose::Catalog).and_then(|(stream, _)| read_limited(stream, FEED_LIMIT))
        .and_then(|data| parse_feed(&data).map(|entries| (data, entries)));
    match downloaded {
        Ok((data, entries)) => {
            if let Some(path) = &cache { let _ = write_atomically(path, &data, 0o600); }
            (Arc::new(entries), FEED_TTL)
        },
        // Offline or the site is down: the old copy is better than nothing.
        Err(_) => (Arc::new(read_cache().unwrap_or_default()), FEED_RETRY),
    }
}

/// The AppImage catalogue. The first call may take a few seconds (it is loaded in the background
/// when the app starts); later calls are instant.
pub fn catalog() -> Arc<Vec<Entry>> {
    let mut guard = FEED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(feed) = guard.as_ref() { if feed.loaded.elapsed() < feed.ttl { return feed.entries.clone(); } }
    let (entries, ttl) = load_feed();
    *guard = Some(Feed { loaded: Instant::now(), ttl, entries: entries.clone() });
    entries
}

/// Load the catalogue ahead of time (called from a background thread at start-up).
pub fn warm_up() { let _ = catalog(); }

/// `Some(false)` when the catalogue was tried and is empty (no connection and no saved copy);
/// `None` while it is still loading.
pub fn catalog_loaded() -> Option<bool> {
    let guard = FEED.try_lock().ok()?;
    guard.as_ref().map(|feed| !feed.entries.is_empty())
}

fn find_entry(id: &str) -> Option<Entry> { catalog().iter().find(|entry| id_key(&entry.id) == id_key(id)).cloned() }

fn package_from_entry(entry: &Entry, installed: Option<&Manifest>) -> Package {
    Package {
        id: entry.id.clone(), name: entry.name.clone(), summary: entry.summary.clone(),
        version: installed.map(|manifest| manifest.version.clone()).unwrap_or_default(), source: Source::Appimage,
        installed: installed.is_some(), update_available: false, remote: None, scope: Some("user".into()),
        publisher: entry.author.clone(), channel: None, confinement: None, kind: "application".into(),
        icon_url: entry.icon.clone(), app_id: None, categories: entry.categories.iter().map(|c| c.to_string()).collect(),
    }
}

fn package_from_manifest(paths: &Paths, manifest: &Manifest) -> Package {
    let icon = manifest.icon.as_ref().and_then(|name| crate::catalog::image_in(&paths.app_dir(&manifest.id).join(name), &paths.base));
    Package {
        id: manifest.id.clone(), name: manifest.name.clone(), summary: manifest.summary.clone(), version: manifest.version.clone(),
        source: Source::Appimage, installed: true, update_available: false, remote: None, scope: Some("user".into()),
        publisher: None, channel: None, confinement: None, kind: "application".into(), icon_url: icon, app_id: None, categories: manifest.categories.clone(),
    }
}

/// Catalogue entries as packages, with the installed ones marked, for the store pages.
pub fn catalog_packages() -> Vec<Package> {
    let installed: HashMap<String, Manifest> = Paths::system().map(|paths| list_manifests(&paths)).unwrap_or_default().into_iter().map(|m| (id_key(&m.id), m)).collect();
    catalog().iter().map(|entry| package_from_entry(entry, installed.get(&id_key(&entry.id)))).collect()
}

pub fn search(query: &str) -> Result<Vec<Package>> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() { return Ok(Vec::new()); }
    let paths = Paths::system()?;
    let installed: HashMap<String, Manifest> = list_manifests(&paths).into_iter().map(|m| (id_key(&m.id), m)).collect();
    let needle = categories::name_key(query);
    let mut hits: Vec<(u8, &Entry)> = Vec::new();
    let catalog = catalog();
    for entry in catalog.iter() {
        let haystack = format!("{} {} {}", entry.name, entry.id, entry.summary).to_lowercase();
        if !words.iter().all(|word| haystack.contains(word.as_str())) { continue; }
        let rank = if entry.name_key == needle { 0 } else if entry.name_key.starts_with(&needle) { 1 } else if entry.name_key.contains(&needle) { 2 } else { 3 };
        hits.push((rank, entry));
    }
    hits.sort_by_key(|(rank, entry)| (*rank, !installed.contains_key(&id_key(&entry.id)), entry.name_key.clone()));
    Ok(hits.into_iter().take(SEARCH_LIMIT).map(|(_, entry)| package_from_entry(entry, installed.get(&id_key(&entry.id)))).collect())
}

pub fn installed() -> Result<Vec<Package>> {
    let paths = Paths::system()?;
    let cached = cached_tags();
    Ok(list_manifests(&paths).iter().map(|manifest| {
        let mut package = package_from_manifest(&paths, manifest);
        package.update_available = cached.get(&id_key(&manifest.id)).is_some_and(|tag| *tag != manifest.version);
        package
    }).collect())
}

pub fn details(package: Package) -> Result<PackageDetails> {
    if !valid_id(&package.id) { return Err(tr!("Invalid AppImage identifier.")); }
    let paths = Paths::system()?;
    let manifest = read_manifest(&paths, &package.id);
    let entry = find_entry(&package.id);
    let mut result = match (&manifest, &entry) {
        (Some(manifest), _) => package_from_manifest(&paths, manifest),
        (None, Some(entry)) => package_from_entry(entry, None),
        (None, None) => return Err(tr!("This app is not in the AppImage catalog.")),
    };
    if result.icon_url.is_none() { result.icon_url = entry.as_ref().and_then(|entry| entry.icon.clone()); }
    if result.publisher.is_none() { result.publisher = entry.as_ref().and_then(|entry| entry.author.clone()); }
    if let Some(manifest) = &manifest {
        result.update_available = cached_tags().get(&id_key(&manifest.id)).is_some_and(|tag| *tag != manifest.version);
    }
    let size = manifest.as_ref().and_then(|manifest| fs::metadata(paths.app_dir(&manifest.id).join(&manifest.file)).ok())
        .map(|meta| tr!("{} MiB installed", format!("{:.1}", meta.len() as f64 / 1_048_576.0)));
    let description = entry.as_ref().map(|entry| entry.description.clone()).filter(|text| !text.is_empty()).unwrap_or_else(|| result.summary.clone());
    Ok(PackageDetails {
        homepage: Some(format!("https://github.com/{}", result.id)),
        license: entry.and_then(|entry| entry.license),
        package: result, description, size,
    })
}

// ---- Releases -------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct Release { tag: String, asset: String, size: u64, url: String, sha256: Option<String> }

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}
#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    state: String,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

const ARCH_TOKENS: &[(&str, &[&str])] = &[
    ("x86_64", &["x86_64", "x86-64", "amd64", "x64"]),
    ("aarch64", &["aarch64", "arm64", "armv8"]),
    ("arm", &["armhf", "armv7", "armv6", "arm32"]),
    ("x86", &["i386", "i686", "ia32", "x86_32"]),
];

fn arch_of(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    // `x86_64` contains `x86` and `arm64` contains `arm`, so look for the longest tokens first.
    let mut found: Option<(&'static str, usize)> = None;
    for (arch, tokens) in ARCH_TOKENS {
        for token in *tokens {
            if lower.contains(token) && found.map(|(_, length)| token.len() > length).unwrap_or(true) { found = Some((arch, token.len())); }
        }
    }
    found.map(|(arch, _)| arch)
}

/// The AppImage to install from a release for `arch` (`std::env::consts::ARCH`): the one named for this
/// architecture, or, on x86-64 only, the only AppImage that names no architecture.
fn pick_asset<'a>(assets: &'a [ApiAsset], arch: &str) -> Option<&'a ApiAsset> {
    let candidates: Vec<&ApiAsset> = assets.iter().filter(|asset| asset.state == "uploaded" && asset.size > 0 && asset.size <= APP_LIMIT
        && asset.name.to_ascii_lowercase().ends_with(".appimage")).collect();
    let tagged: Vec<&&ApiAsset> = candidates.iter().filter(|asset| arch_of(&asset.name) == Some(arch)).collect();
    if let Some(best) = tagged.iter().min_by_key(|asset| asset.name.len()) { return Some(**best); }
    let untagged: Vec<&&ApiAsset> = candidates.iter().filter(|asset| arch_of(&asset.name).is_none()).collect();
    if arch == "x86_64" && untagged.len() == 1 { return Some(*untagged[0]); }
    None
}

fn parse_digest(digest: Option<&str>) -> Option<String> {
    let value = digest?.strip_prefix("sha256:")?.to_ascii_lowercase();
    (value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())).then_some(value)
}

fn release_from(id: &str, data: &[u8], arch: &str) -> Result<Release> {
    let release: ApiRelease = serde_json::from_slice(data).map_err(|e| tr!("GitHub's answer is not valid: {}", e))?;
    let tag = clean_text(&release.tag_name, 100);
    if release.draft || tag.is_empty() { return Err(tr!("This project has no published release.")); }
    let asset = pick_asset(&release.assets, arch)
        .ok_or_else(|| tr!("The latest release has no AppImage for this computer ({}).", arch))?;
    let path = checked_url(&asset.browser_download_url, Purpose::Download)?;
    let expected = format!("/{}/releases/download/", id_key(id));
    if !path.to_ascii_lowercase().starts_with(&expected) { return Err(tr!("The download address does not belong to this project's releases.")); }
    Ok(Release { tag, asset: sanitize_file_name(&asset.name), size: asset.size, url: asset.browser_download_url.clone(), sha256: parse_digest(asset.digest.as_deref()) })
}

static RELEASES: Mutex<Option<HashMap<String, (Instant, Release)>>> = Mutex::new(None);

fn cached_tags() -> HashMap<String, String> {
    let guard = RELEASES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.as_ref().map(|map| map.iter().filter(|(_, (when, _))| when.elapsed() < RELEASE_TTL).map(|(id, (_, release))| (id.clone(), release.tag.clone())).collect()).unwrap_or_default()
}

/// The latest stable release of a project, remembered for a few minutes so that checking for updates
/// and then opening the confirmation does not spend GitHub's request allowance twice.
fn latest_release(id: &str) -> Result<Release> {
    if !valid_id(id) { return Err(tr!("Invalid AppImage identifier.")); }
    let key = id_key(id);
    if let Some((when, release)) = RELEASES.lock().unwrap_or_else(|p| p.into_inner()).as_ref().and_then(|map| map.get(&key)) {
        if when.elapsed() < RELEASE_TTL { return Ok(release.clone()); }
    }
    let (stream, _) = open_url(&format!("https://api.github.com/repos/{id}/releases/latest"), Purpose::Api)
        .map_err(|e| if e == tr!("Not found on GitHub (error 404).") { tr!("This project has no published release.") } else { e })?;
    let release = release_from(id, &read_limited(stream, API_LIMIT)?, std::env::consts::ARCH)?;
    RELEASES.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(HashMap::new).insert(key, (Instant::now(), release.clone()));
    Ok(release)
}

/// Forget the remembered releases, so the next look at GitHub is a fresh one (an update check).
pub fn forget_releases() { *RELEASES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None; }

pub fn updates() -> Result<Vec<Package>> {
    let paths = Paths::system()?;
    let mut result = Vec::new();
    for manifest in list_manifests(&paths) {
        // The first failure (usually: no network) ends the check instead of waiting for every app.
        let Ok(release) = latest_release(&manifest.id) else { break };
        if release.tag != manifest.version {
            let mut package = package_from_manifest(&paths, &manifest);
            package.update_available = true;
            package.version = release.tag;
            result.push(package);
        }
    }
    Ok(result)
}

// ---- Plans ----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind { Install, Update, Remove }

/// A confirmed-by-the-user plan, kept until it is executed.
#[derive(Debug)]
pub struct Prepared {
    kind: Kind,
    id: String,
    name: String,
    entry: Option<Entry>,
    release: Option<Release>,
    manifest: Option<Manifest>,
}

impl Prepared {
    /// What will happen, as shown in the "technical details" of the confirmation.
    pub fn describe(&self) -> String {
        let place = Paths::system().map(|paths| paths.app_dir(&self.id).display().to_string()).unwrap_or_default();
        match (&self.kind, &self.release) {
            (Kind::Remove, _) | (_, None) => format!("remove {place}\nremove launcher {}", launcher_name(&self.id)),
            (_, Some(release)) => format!("download {}\n  size {} bytes, SHA-256 {}\nsave as {place}/{}\nchmod 755, add launcher {}",
                release.url, release.size, release.sha256.as_deref().unwrap_or("(not published by GitHub)"), release.asset, launcher_name(&self.id)),
        }
    }
}

fn mebibytes(bytes: u64) -> String { format!("{:.1}", bytes as f64 / 1_048_576.0) }

const FUSE_LIBRARIES: [&str; 8] = [
    "/usr/lib/x86_64-linux-gnu/libfuse.so.2", "/lib/x86_64-linux-gnu/libfuse.so.2", "/usr/lib/aarch64-linux-gnu/libfuse.so.2",
    "/lib/aarch64-linux-gnu/libfuse.so.2", "/usr/lib/arm-linux-gnueabihf/libfuse.so.2", "/usr/lib/i386-linux-gnu/libfuse.so.2",
    "/usr/lib64/libfuse.so.2", "/usr/lib/libfuse.so.2",
];
/// AppImages mount themselves with FUSE 2. Without it they still run, by unpacking first.
fn fuse2_available() -> bool { FUSE_LIBRARIES.iter().any(|path| Path::new(path).exists()) }

pub fn plan(package: &Package, action: &str) -> Result<(Prepared, Vec<String>)> {
    if !valid_id(&package.id) { return Err(tr!("Invalid AppImage identifier.")); }
    let paths = Paths::system()?;
    let manifest = read_manifest(&paths, &package.id);
    let confirmation = tr!("The confirmation is valid for 2 minutes and a single run. The release is checked again before anything is installed.");
    match action {
        "remove" => {
            let manifest = manifest.ok_or_else(|| tr!("This AppImage is not installed."))?;
            let directory = paths.app_dir(&manifest.id).display().to_string();
            let details = vec![
                tr!("Deletes the AppImage and its launcher: {}. Nothing else is removed.", directory),
                tr!("The app's own settings and documents, which live elsewhere in your home folder, are kept."),
                confirmation,
            ];
            Ok((Prepared { kind: Kind::Remove, id: manifest.id.clone(), name: manifest.name.clone(), entry: None, release: None, manifest: Some(manifest) }, details))
        },
        "install" | "update" => {
            let installing = action == "install";
            if installing && manifest.is_some() { return Err(tr!("This AppImage is already installed.")); }
            if !installing && manifest.is_none() { return Err(tr!("This AppImage is not installed.")); }
            let entry = find_entry(&package.id);
            if installing && entry.is_none() { return Err(tr!("This app is not in the AppImage catalog.")); }
            let release = latest_release(&package.id)?;
            if let Some(old) = &manifest {
                if old.version == release.tag { return Err(tr!("No update available for this AppImage.")); }
            }
            let name = entry.as_ref().map(|e| e.name.clone()).or_else(|| manifest.as_ref().map(|m| m.name.clone())).unwrap_or_else(|| package.name.clone());
            let author = entry.as_ref().and_then(|e| e.author.clone()).unwrap_or_else(|| package.id.split('/').next().unwrap_or("").to_string());
            let mut details = vec![
                tr!("Downloads {} ({} MiB), release {} of github.com/{}, over https.", release.asset, mebibytes(release.size), release.tag, package.id),
                match &release.sha256 {
                    Some(digest) => tr!("GitHub publishes this checksum for the file (SHA-256): {}. The download is discarded if it does not match.", digest),
                    None => tr!("GitHub publishes no checksum for this file, so its integrity rests on the https connection to GitHub. The SHA-256 of what is downloaded is shown in the result."),
                },
                tr!("An AppImage is not sandboxed: the app runs with all the permissions of your user and can read and change your files. Install it only if you trust {}.", author),
                tr!("It is saved in {} and a launcher is added to your application menu. Everything stays in your home folder: no administrator password is needed.", paths.app_dir(&package.id).display()),
            ];
            if !fuse2_available() {
                details.push(tr!("FUSE 2 is not installed on this computer, so the app will start by unpacking itself first (a little slower to open)."));
            }
            if let Some(old) = &manifest {
                details.push(tr!("Replaces version {} with {}. The old file is deleted once the new one is in place.", old.version, release.tag));
            }
            details.push(confirmation);
            let kind = if installing { Kind::Install } else { Kind::Update };
            Ok((Prepared { kind, id: package.id.clone(), name, entry, release: Some(release), manifest }, details))
        },
        _ => Err(tr!("Invalid action.")),
    }
}

// ---- Execution ------------------------------------------------------------------------------

/// `Exec=` and `Icon=` values must not need escaping; anything unusual means "no launcher".
fn plain_path(path: &Path) -> Option<&str> {
    let text = path.to_str()?;
    (text.starts_with('/') && text.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._+-~@".contains(&b))).then_some(text)
}

/// The text of the launcher in the application menu, or `None` when the path is not plain enough to
/// write into a `.desktop` file safely.
fn launcher_text(id: &str, name: &str, summary: &str, version: &str, program: &Path, icon: Option<&Path>, freedesktop: &[String], extract_and_run: bool) -> Option<String> {
    let program = plain_path(program)?;
    let icon = match icon { Some(path) => plain_path(path)?.to_string(), None => "application-x-executable".to_string() };
    let mut categories: Vec<&str> = freedesktop.iter().map(String::as_str).filter(|c| c.bytes().all(|b| b.is_ascii_alphanumeric())).collect();
    if categories.is_empty() { categories.push("Utility"); }
    let exec = if extract_and_run { format!("env APPIMAGE_EXTRACT_AND_RUN=1 {program} %U") } else { format!("{program} %U") };
    Some(format!("[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec={exec}\nIcon={icon}\nTerminal=false\nCategories={};\nX-AppImage-Version={}\n{MARKER}={id}\n",
        clean_text(name, 80), clean_text(summary, 140), categories.join(";"), clean_text(version, 60)))
}

/// What was read from the network: size, SHA-256 and the first bytes, for the checks that follow.
struct Received { size: u64, sha256: String, head: Vec<u8> }

fn copy_verified(mut input: impl Read, output: &mut impl Write, expected: u64, deadline: Instant) -> Result<Received> {
    let mut hasher = Sha256::new();
    let mut head = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        if Instant::now() > deadline { return Err(tr!("The download took too long and was stopped.")); }
        let count = input.read(&mut buffer).map_err(|e| tr!("Network error: {}", e))?;
        if count == 0 { break; }
        size += count as u64;
        if size > expected || size > APP_LIMIT { return Err(tr!("The file is larger than announced by GitHub; the download was stopped.")); }
        if head.len() < 16 { head.extend_from_slice(&buffer[..count.min(16 - head.len())]); }
        hasher.update(&buffer[..count]);
        output.write_all(&buffer[..count]).map_err(|e| tr!("Cannot write the file: {}", e))?;
    }
    if size != expected { return Err(tr!("The download is incomplete: {} of {} bytes.", size, expected)); }
    Ok(Received { size, sha256: hex(&hasher.finalize()), head })
}

/// The ELF header followed by the `AI` + type 1 or 2 marker that every AppImage carries.
fn looks_like_appimage(head: &[u8]) -> bool {
    head.len() >= 11 && head[..4] == [0x7f, b'E', b'L', b'F'] && head[8] == b'A' && head[9] == b'I' && matches!(head[10], 1 | 2)
}

fn random_name() -> String {
    let mut random = [0u8; 8];
    let _ = getrandom::getrandom(&mut random);
    hex(&random)
}

fn download_icon(url: &str, destination: &Path, fetch: Fetch) -> Result<()> {
    let (stream, _) = fetch(url, Purpose::Catalog)?;
    let data = read_limited(stream, ICON_LIMIT)?;
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") { return Err(tr!("The icon is not a PNG image.")); }
    write_atomically(destination, &data, 0o644).map_err(|e| tr!("Cannot write the file: {}", e))
}

fn install(paths: &Paths, prepared: &Prepared, fetch: Fetch, extract_and_run: bool) -> Result<String> {
    let release = prepared.release.as_ref().ok_or_else(|| tr!("Missing release."))?;
    checked_url(&release.url, Purpose::Download)?;
    let dir = paths.app_dir(&prepared.id);
    let previous = read_manifest(paths, &prepared.id);
    match (&prepared.kind, &previous) {
        (Kind::Install, Some(_)) => return Err(tr!("This AppImage is already installed.")),
        (Kind::Update, None) => return Err(tr!("This AppImage is not installed.")),
        (Kind::Update, Some(old)) if Some(old) != prepared.manifest.as_ref() => return Err(tr!("The installed AppImage has changed. Prepare a new plan.")),
        _ => {},
    }
    let created = !dir.exists();
    fs::create_dir_all(&dir).map_err(|e| tr!("Cannot create the folder: {}", e))?;
    if !real_dir(&dir) { return Err(tr!("The install folder is not valid.")); }
    let partial = dir.join(format!(".download-{}.part", random_name()));
    let outcome = (|| -> Result<(Received, PathBuf)> {
        let (stream, announced) = fetch(&release.url, Purpose::Download)?;
        if announced.is_some_and(|length| length != release.size) { return Err(tr!("The file size differs from the one GitHub announced.")); }
        let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&partial).map_err(|e| tr!("Cannot create the file: {}", e))?;
        let received = copy_verified(stream, &mut file, release.size, Instant::now() + DOWNLOAD_TIMEOUT)?;
        file.sync_all().map_err(|e| tr!("Cannot write the file: {}", e))?;
        if let Some(expected) = &release.sha256 {
            if *expected != received.sha256 { return Err(tr!("The checksum of the download does not match the one published by GitHub; the file was discarded.")); }
        }
        if !looks_like_appimage(&received.head) { return Err(tr!("The downloaded file is not an AppImage; it was discarded.")); }
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o755)).map_err(|e| tr!("Cannot write the file: {}", e))?;
        let target = dir.join(&release.asset);
        fs::rename(&partial, &target).map_err(|e| tr!("Cannot write the file: {}", e))?;
        Ok((received, target))
    })();
    let (received, target) = match outcome {
        Ok(done) => done,
        Err(error) => {
            let _ = fs::remove_file(&partial);
            if created { let _ = fs::remove_dir(&dir); }
            return Err(error);
        },
    };

    let entry = prepared.entry.as_ref();
    let mut notes: Vec<String> = Vec::new();
    // The icon is optional: the app works and appears in the menu without it.
    let kept_icon = previous.as_ref().and_then(|old| old.icon.clone()).filter(|name| regular_file(&dir.join(name)));
    let icon_name = match (kept_icon, entry.and_then(|e| e.icon.as_deref())) {
        (Some(name), _) => Some(name),
        (None, Some(url)) => match download_icon(url, &dir.join("icon.png"), fetch) {
            Ok(()) => Some("icon.png".to_string()),
            Err(_) => { notes.push(tr!("The icon could not be downloaded; the app uses a generic one.")); None },
        },
        (None, None) => None,
    };
    let icon_path = icon_name.as_ref().map(|name| dir.join(name));
    let freedesktop = entry.map(|e| e.freedesktop.clone()).unwrap_or_default();
    let summary = entry.map(|e| e.summary.clone()).or_else(|| previous.as_ref().map(|m| m.summary.clone())).unwrap_or_default();
    let categories: Vec<String> = entry.map(|e| e.categories.iter().map(|c| c.to_string()).collect()).or_else(|| previous.as_ref().map(|m| m.categories.clone())).unwrap_or_default();
    let launcher = paths.launchers.join(launcher_name(&prepared.id));
    match launcher_text(&prepared.id, &prepared.name, &summary, &release.tag, &target, icon_path.as_deref(), &freedesktop, extract_and_run) {
        Some(text) => match write_atomically(&launcher, text.as_bytes(), 0o644) {
            Ok(()) => notes.push(tr!("Launcher added to the application menu: {}", launcher.display())),
            Err(e) => notes.push(tr!("The launcher could not be created ({}); open the app from Nexa Store.", e)),
        },
        None => notes.push(tr!("The install path has unusual characters, so no launcher was created; open the app from Nexa Store.")),
    }
    let manifest = Manifest {
        id: prepared.id.clone(), name: prepared.name.clone(), version: release.tag.clone(), file: release.asset.clone(), sha256: received.sha256.clone(),
        url: release.url.clone(), summary, icon: icon_name, categories, desktop: launcher_name(&prepared.id),
        installed_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    };
    let record = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    write_atomically(&dir.join("meta.json"), &record, 0o644).map_err(|e| tr!("Cannot write the file: {}", e))?;
    if let Some(old) = &previous {
        if old.file != release.asset { let _ = fs::remove_file(dir.join(&old.file)); }
    }
    let mut lines = vec![
        tr!("Downloaded {} ({} MiB) from release {}.", release.asset, mebibytes(received.size), release.tag),
        format!("SHA-256: {}", received.sha256),
        if release.sha256.is_some() { tr!("The checksum matches the one published by GitHub.") } else { tr!("GitHub published no checksum for this file; compare the SHA-256 above with the project's page if you want to be sure.") },
        tr!("Installed in {}", dir.display()),
    ];
    lines.extend(notes);
    Ok(lines.join("\n"))
}

fn remove(paths: &Paths, prepared: &Prepared) -> Result<String> {
    let approved = prepared.manifest.as_ref().ok_or_else(|| tr!("This AppImage is not installed."))?;
    let current = read_manifest(paths, &prepared.id).ok_or_else(|| tr!("This AppImage is not installed."))?;
    if &current != approved { return Err(tr!("The installed AppImage has changed. Prepare a new plan.")); }
    let dir = paths.app_dir(&prepared.id);
    if !real_dir(&dir) || !dir.starts_with(&paths.base) { return Err(tr!("The install folder is not valid.")); }
    let launcher = paths.launchers.join(launcher_name(&prepared.id));
    let mut lines = Vec::new();
    // Only a launcher this module wrote for this very app is removed.
    let ours = regular_file(&launcher) && fs::read_to_string(&launcher).map(|text| text.lines().any(|line| line == format!("{MARKER}={}", prepared.id) || line.eq_ignore_ascii_case(&format!("{MARKER}={}", current.id)))).unwrap_or(false);
    if ours { let _ = fs::remove_file(&launcher); lines.push(tr!("Launcher removed from the application menu.")); }
    fs::remove_dir_all(&dir).map_err(|e| tr!("Cannot delete the folder: {}", e))?;
    if let Some(owner) = dir.parent() { let _ = fs::remove_dir(owner); }
    lines.insert(0, tr!("Deleted {}", dir.display()));
    Ok(lines.join("\n"))
}

/// Run a prepared plan. Returns whether it worked and the text to show.
pub fn execute(prepared: &Prepared) -> (bool, String) {
    let result = Paths::system().and_then(|paths| match prepared.kind {
        Kind::Install | Kind::Update => install(&paths, prepared, &open_url, !fuse2_available()),
        Kind::Remove => remove(&paths, prepared),
    });
    match result { Ok(text) => (true, text), Err(error) => (false, error) }
}

/// Start an installed AppImage, detached from this process. If it ends at once with an error (for
/// example it needs a library that is missing) the error is reported instead of failing silently.
pub fn open(package: &Package) -> Result<()> { open_installed(&Paths::system()?, &package.id) }

fn open_installed(paths: &Paths, id: &str) -> Result<()> {
    let manifest = read_manifest(paths, id).ok_or_else(|| tr!("This AppImage does not appear to be installed."))?;
    let program = paths.app_dir(&manifest.id).join(&manifest.file);
    let mut command = Command::new(&program);
    if !fuse2_available() { command.env("APPIMAGE_EXTRACT_AND_RUN", "1"); }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from).filter(|path| path.is_dir()) { command.current_dir(home); }
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0)
        .spawn().map_err(|e| tr!("Cannot start the app: {}", e))?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(1500) {
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => return Err(tr!("The app closed right after starting ({}). It may need a library that is not installed.", status)),
            Ok(Some(_)) => return Ok(()),
            _ => thread::sleep(Duration::from_millis(50)),
        }
    }
    // Reap it when it ends so no zombie is left behind; the app itself keeps running on its own.
    thread::spawn(move || { let _ = child.wait(); });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn temporary(label: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("nexa-appimage-test-{label}-{}-{}", std::process::id(), random_name()));
        fs::create_dir_all(&root).unwrap();
        Paths { base: root.join("data/nexa-store/appimages"), launchers: root.join("data/applications") }
    }
    fn cleanup(paths: &Paths) { let _ = fs::remove_dir_all(paths.base.ancestors().nth(3).unwrap()); }

    /// A file that has the header of an AppImage (type 2) followed by `padding` bytes.
    fn image(padding: usize) -> Vec<u8> {
        let mut data = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0, b'A', b'I', 2, 0];
        data.extend(std::iter::repeat(7u8).take(padding));
        data
    }
    fn sha(data: &[u8]) -> String { hex(&Sha256::digest(data)) }

    fn release_for(data: &[u8], digest: bool) -> Release {
        Release { tag: "v1.2.0".into(), asset: "Demo-1.2.0-x86_64.AppImage".into(), size: data.len() as u64,
            url: "https://github.com/octo/demo/releases/download/v1.2.0/Demo-1.2.0-x86_64.AppImage".into(), sha256: digest.then(|| sha(data)) }
    }
    fn entry() -> Entry {
        Entry { id: "octo/demo".into(), name: "Demo".into(), description: "A demo app".into(), summary: "A demo app".into(), categories: vec!["utilities"],
            freedesktop: vec!["Utility".into()], license: Some("MIT".into()), author: Some("octo".into()), icon: Some("https://appimage.github.io/database/Demo/icon.png".into()), name_key: "demo".into() }
    }
    fn prepared(kind: Kind, release: Option<Release>, manifest: Option<Manifest>) -> Prepared {
        Prepared { kind, id: "octo/demo".into(), name: "Demo".into(), entry: Some(entry()), release, manifest }
    }
    fn serve<'a>(app: &'a [u8], icon: Option<&'a [u8]>, announced: Option<u64>) -> impl Fn(&str, Purpose) -> Result<(Stream, Option<u64>)> + 'a {
        move |url, purpose| {
            checked_url(url, purpose)?;
            if purpose == Purpose::Download { Ok((Box::new(Cursor::new(app.to_vec())), announced.or(Some(app.len() as u64)))) }
            else { match icon { Some(data) => Ok((Box::new(Cursor::new(data.to_vec())), None)), None => Err("offline".into()) } }
        }
    }
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000";

    #[test]
    fn identifiers_are_github_owner_and_repo_only() {
        for good in ["probonopd/appimagetool", "Foglio24/nexa-store", "a/b", "org.name/repo_1.2"] { assert!(valid_id(good), "{good}"); }
        for bad in ["", "owner", "owner/", "/repo", "../etc/passwd", "owner/../x", "owner/repo/extra", "-owner/repo", ".hidden/repo", "owner/.repo", "ow ner/repo", "owner/repo;id", "owner/repo\n", "https://github.com/a/b", &format!("{}/x", "a".repeat(40))] {
            assert!(!valid_id(bad), "{bad:?}");
        }
        assert!(safe_file_name("App-1.0_x86_64.AppImage") && safe_file_name("tool.appimage"));
        for bad in ["", ".hidden.AppImage", "a/b.AppImage", "../a.AppImage", "a b.AppImage", "a.AppImage.zsync", "run.sh", "a;id.AppImage"] { assert!(!safe_file_name(bad), "{bad:?}"); }
        assert_eq!(sanitize_file_name("My App (x64).AppImage"), "My_App__x64_.AppImage");
        assert!(safe_file_name(&sanitize_file_name("../../etc/passwd.AppImage")) || sanitize_file_name("../../etc/passwd.AppImage").starts_with("_"));
    }

    #[test]
    fn only_github_over_https_is_allowed_including_redirect_targets() {
        assert_eq!(checked_url("https://github.com/o/r/releases/download/v1/a.AppImage", Purpose::Download).unwrap(), "/o/r/releases/download/v1/a.AppImage");
        assert!(checked_url("https://release-assets.githubusercontent.com/github-production-release-asset/1?x=y", Purpose::Download).is_ok());
        assert!(checked_url("https://objects.githubusercontent.com/a", Purpose::Download).is_ok());
        assert!(checked_url("https://api.github.com/repos/o/r/releases/latest", Purpose::Api).is_ok());
        assert!(checked_url("https://appimage.github.io/feed.json", Purpose::Catalog).is_ok());
        for bad in ["http://github.com/a", "ftp://github.com/a", "https://evil.com/a", "https://github.com.evil.com/a", "https://evilgithub.com/a", "https://githubusercontent.com/a",
            "https://.githubusercontent.com/a", "https://user@github.com/a", "https://github.com:444/a", "https://github.com\\@evil.com/a", "https://github.com/a b", "https://github.com/a\n", "//github.com/a", "https://"] {
            assert!(checked_url(bad, Purpose::Download).is_err(), "{bad:?}");
        }
        assert!(checked_url("https://github.com/a", Purpose::Api).is_err(), "each purpose has its own hosts");
        assert!(checked_url("https://api.github.com/a", Purpose::Download).is_err());
    }

    #[test]
    fn the_feed_keeps_only_valid_github_projects() {
        let feed = br#"{"version":1,"items":[
          {"name":"Good App","description":"Does   things\n well","categories":["Graphics","Qt",null,"Utility"],"authors":[{"name":"Ann","url":"https://x"}],"license":"MIT",
           "links":[{"type":"GitHub","url":"ann/good-app"},{"type":"Download","url":"https://github.com/ann/good-app/releases"}],"icons":["Good/icons/512x512/good.png"]},
          {"name":"No GitHub","links":[{"type":"Download","url":"https://example.com/x"}]},
          {"name":"Bad id","links":[{"type":"GitHub","url":"../../x"}]},
          {"name":"","links":[{"type":"GitHub","url":"a/b"}]},
          {"name":"Duplicate","links":[{"type":"GitHub","url":"ANN/Good-App"}]},
          {"name":"Svg icon","categories":null,"icons":["x/icon.svg"],"links":[{"type":"GitHub","url":"o/svg"}]},
          {"name":"Traversal icon","icons":["../../etc/passwd.png"],"links":[{"type":"GitHub","url":"o/trav"}]},
          {"name":7757,"license":12,"description":["odd"],"authors":"x","categories":[1,"Game"],"links":[{"type":"GitHub","url":"o/odd"}]},
          "not an object", {"name":"No links"}
        ]}"#;
        let entries = parse_feed(feed).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["7757", "Good App", "Svg icon", "Traversal icon"], "odd values are read tolerantly, broken items skipped");
        let good = entries.iter().find(|e| e.name == "Good App").unwrap();
        assert_eq!((good.id.as_str(), good.summary.as_str(), good.author.as_deref(), good.license.as_deref()), ("ann/good-app", "Does things well", Some("Ann"), Some("MIT")));
        assert_eq!(good.categories, vec!["graphics", "utilities"]);
        assert_eq!(good.icon.as_deref(), Some("https://appimage.github.io/database/Good/icons/512x512/good.png"));
        assert!(entries.iter().filter(|e| e.name == "Svg icon" || e.name == "Traversal icon").all(|e| e.icon.is_none()), "only PNG icons without path tricks");
        let odd = entries.iter().find(|e| e.name == "7757").unwrap();
        assert_eq!((odd.license.as_deref(), odd.categories.clone(), odd.author.clone()), (Some("12"), vec!["games"], None));
        assert!(parse_feed(b"not json").is_err());
        assert!(parse_feed(br#"{"items":[]}"#).unwrap().is_empty());
    }

    fn asset(name: &str) -> ApiAsset {
        ApiAsset { name: name.into(), size: 1000, state: "uploaded".into(), browser_download_url: format!("https://github.com/o/r/releases/download/v1/{name}"), digest: None }
    }

    #[test]
    fn the_asset_matches_this_computers_architecture() {
        let assets = vec![asset("App-1.0-aarch64.AppImage"), asset("App-1.0-x86_64.AppImage"), asset("App-1.0-x86_64.AppImage.zsync"), asset("App-1.0.deb"), asset("App-1.0-i386.AppImage")];
        assert_eq!(pick_asset(&assets, "x86_64").unwrap().name, "App-1.0-x86_64.AppImage");
        assert_eq!(pick_asset(&assets, "aarch64").unwrap().name, "App-1.0-aarch64.AppImage");
        assert!(pick_asset(&assets, "riscv64").is_none());
        let untagged = vec![asset("App.AppImage"), asset("App.dmg")];
        assert_eq!(pick_asset(&untagged, "x86_64").unwrap().name, "App.AppImage", "an AppImage that names no architecture is taken as x86-64");
        assert!(pick_asset(&untagged, "aarch64").is_none(), "but never guessed for other architectures");
        assert!(pick_asset(&[asset("A.AppImage"), asset("B.AppImage")], "x86_64").is_none(), "two ambiguous files: refuse to guess");
        let mut pending = asset("App-x86_64.AppImage"); pending.state = "starter".into();
        assert!(pick_asset(&[pending], "x86_64").is_none());
        let mut empty = asset("App-x86_64.AppImage"); empty.size = 0;
        assert!(pick_asset(&[empty], "x86_64").is_none());
        assert_eq!(arch_of("tool-linux-x64.AppImage"), Some("x86_64"));
        assert_eq!(arch_of("tool-arm64.AppImage"), Some("aarch64"));
        assert_eq!(arch_of("tool-x86_64.AppImage"), Some("x86_64"), "x86_64 is not mistaken for x86");
        assert_eq!(arch_of("tool.AppImage"), None);
    }

    #[test]
    fn a_release_is_accepted_only_with_a_download_address_inside_the_project() {
        let digest = "a".repeat(64);
        let body = |url: &str| format!(r#"{{"tag_name":"v2.0","draft":false,"assets":[{{"name":"Tool-x86_64.AppImage","size":5000,"state":"uploaded","browser_download_url":"{url}","digest":"sha256:{digest}"}}]}}"#);
        let release = release_from("o/r", body("https://github.com/o/r/releases/download/v2.0/Tool-x86_64.AppImage").as_bytes(), "x86_64").unwrap();
        assert_eq!((release.tag.as_str(), release.size, release.sha256.as_deref()), ("v2.0", 5000, Some(digest.as_str())));
        assert!(release_from("O/R", body("https://github.com/o/r/releases/download/v2.0/Tool-x86_64.AppImage").as_bytes(), "x86_64").is_ok(), "GitHub names are case-insensitive");
        for url in ["https://github.com/other/project/releases/download/v2.0/Tool-x86_64.AppImage", "https://github.com/o/r/archive/v2.0.zip", "https://evil.com/o/r/releases/download/v2.0/x.AppImage", "http://github.com/o/r/releases/download/v2.0/x.AppImage"] {
            assert!(release_from("o/r", body(url).as_bytes(), "x86_64").is_err(), "{url}");
        }
        assert!(release_from("o/r", br#"{"tag_name":"v1","draft":true,"assets":[]}"#, "x86_64").is_err());
        assert!(release_from("o/r", br#"{"tag_name":"v1","assets":[]}"#, "x86_64").is_err(), "no AppImage in the release");
        assert!(release_from("o/r", b"[]", "x86_64").is_err());
        assert_eq!(parse_digest(Some(&format!("sha256:{}", "AB".repeat(32)))), Some("ab".repeat(32)));
        for bad in [None, Some("md5:abc"), Some("sha256:xyz"), Some("sha256:")] { assert_eq!(parse_digest(bad), None, "{bad:?}"); }
    }

    #[test]
    fn the_appimage_header_is_checked() {
        assert!(looks_like_appimage(&image(10)[..16]));
        let mut type_one = image(10); type_one[10] = 1; assert!(looks_like_appimage(&type_one));
        let mut type_three = image(10); type_three[10] = 3; assert!(!looks_like_appimage(&type_three));
        assert!(!looks_like_appimage(b"#!/bin/sh\necho hi\n......"));
        assert!(!looks_like_appimage(b"<html>not found</html>"));
        assert!(!looks_like_appimage(&[]));
        assert!(!looks_like_appimage(&image(0)[..10]));
    }

    #[test]
    fn a_launcher_is_written_only_for_plain_paths_and_never_for_odd_text() {
        let text = launcher_text("o/r", "My App\nExec=evil", "Summary", "v1", Path::new("/home/u/.local/share/nexa-store/appimages/o/r/App.AppImage"),
            Some(Path::new("/home/u/.local/share/nexa-store/appimages/o/r/icon.png")), &["Graphics".into(), "Bad;Cat=1".into()], false).unwrap();
        assert!(text.starts_with("[Desktop Entry]\n") && text.contains("\nExec=/home/u/.local/share/nexa-store/appimages/o/r/App.AppImage %U\n"));
        assert!(text.contains("\nName=My App Exec=evil\n"), "newlines in a name cannot start a new key: {text}");
        assert_eq!(text.lines().filter(|line| line.starts_with("Exec=")).count(), 1);
        assert!(text.contains("\nCategories=Graphics;\n") && text.contains("X-Nexa-Store-AppImage=o/r\n"), "{text}");
        let extract = launcher_text("o/r", "App", "", "v1", Path::new("/data/App.AppImage"), None, &[], true).unwrap();
        assert!(extract.contains("Exec=env APPIMAGE_EXTRACT_AND_RUN=1 /data/App.AppImage %U") && extract.contains("Icon=application-x-executable") && extract.contains("Categories=Utility;"));
        for odd in ["/home/with space/App.AppImage", "/home/q\"uote/App.AppImage", "/home/$x/App.AppImage", "relative/App.AppImage", "/home/a;b/App.AppImage", "/home/é/App.AppImage"] {
            assert!(launcher_text("o/r", "App", "", "v1", Path::new(odd), None, &[], false).is_none(), "{odd}");
        }
    }

    #[test]
    fn install_verifies_the_download_then_records_launcher_and_manifest() {
        let paths = temporary("install");
        let app = image(5000);
        let release = release_for(&app, true);
        let outcome = install(&paths, &prepared(Kind::Install, Some(release.clone()), None), &serve(&app, Some(PNG), None), true).unwrap();
        assert!(outcome.contains(&sha(&app)) && outcome.contains("matches"), "{outcome}");
        let dir = paths.app_dir("octo/demo");
        let file = dir.join("Demo-1.2.0-x86_64.AppImage");
        assert_eq!(fs::read(&file).unwrap(), app);
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(dir.join("icon.png").is_file());
        assert_eq!(fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".part")).count(), 0, "no partial file stays");
        let launcher = fs::read_to_string(paths.launchers.join(launcher_name("octo/demo"))).unwrap();
        assert!(launcher.contains("Name=Demo") && launcher.contains("APPIMAGE_EXTRACT_AND_RUN=1") && launcher.contains("X-Nexa-Store-AppImage=octo/demo"), "{launcher}");
        let manifest = read_manifest(&paths, "OCTO/Demo").expect("ids are case-insensitive");
        assert_eq!((manifest.version.as_str(), manifest.sha256.as_str(), manifest.icon.as_deref()), ("v1.2.0", sha(&app).as_str(), Some("icon.png")));
        assert_eq!(list_manifests(&paths).len(), 1);
        let again = install(&paths, &prepared(Kind::Install, Some(release), None), &serve(&app, Some(PNG), None), false);
        assert!(again.unwrap_err().contains("already installed"));
        cleanup(&paths);
    }

    #[test]
    fn a_bad_download_is_discarded_and_leaves_nothing_behind() {
        let paths = temporary("reject");
        let app = image(2000);
        let dir = paths.app_dir("octo/demo");
        let wrong_digest = Release { sha256: Some("0".repeat(64)), ..release_for(&app, false) };
        let error = install(&paths, &prepared(Kind::Install, Some(wrong_digest), None), &serve(&app, None, None), false).unwrap_err();
        assert!(error.contains("does not match"), "{error}");
        let script = b"#!/bin/sh\necho pwned\n".repeat(50);
        let error = install(&paths, &prepared(Kind::Install, Some(release_for(&script, true)), None), &serve(&script, None, None), false).unwrap_err();
        assert!(error.contains("not an AppImage"), "{error}");
        let bigger = Release { size: app.len() as u64 - 10, ..release_for(&app, false) };
        assert!(install(&paths, &prepared(Kind::Install, Some(bigger), None), &serve(&app, None, Some(app.len() as u64 - 10)), false).unwrap_err().contains("larger than announced"));
        let smaller = Release { size: app.len() as u64 + 10, ..release_for(&app, false) };
        assert!(install(&paths, &prepared(Kind::Install, Some(smaller), None), &serve(&app, None, Some(app.len() as u64 + 10)), false).unwrap_err().contains("incomplete"));
        let lying = release_for(&app, false);
        assert!(install(&paths, &prepared(Kind::Install, Some(lying), None), &serve(&app, None, Some(app.len() as u64 + 1)), false).unwrap_err().contains("differs"));
        let foreign = Release { url: "https://evil.example.com/a.AppImage".into(), ..release_for(&app, false) };
        assert!(install(&paths, &prepared(Kind::Install, Some(foreign), None), &serve(&app, None, None), false).is_err());
        assert!(!dir.exists(), "a folder created for a failed install is removed again");
        assert!(read_manifest(&paths, "octo/demo").is_none() && !paths.launchers.join(launcher_name("octo/demo")).exists());
        cleanup(&paths);
    }

    #[test]
    fn a_missing_icon_does_not_stop_the_install() {
        let paths = temporary("noicon");
        let app = image(100);
        let outcome = install(&paths, &prepared(Kind::Install, Some(release_for(&app, false)), None), &serve(&app, Some(b"<html>nope</html>"), None), false).unwrap();
        assert!(outcome.contains("icon could not be downloaded") && outcome.contains("published no checksum"), "{outcome}");
        assert!(read_manifest(&paths, "octo/demo").unwrap().icon.is_none());
        assert!(!fs::read_to_string(paths.launchers.join(launcher_name("octo/demo"))).unwrap().contains("icon.png"));
        cleanup(&paths);
    }

    #[test]
    fn update_replaces_the_file_and_remove_deletes_only_this_app() {
        let paths = temporary("lifecycle");
        let old_app = image(300);
        install(&paths, &prepared(Kind::Install, Some(release_for(&old_app, true)), None), &serve(&old_app, Some(PNG), None), false).unwrap();
        let installed = read_manifest(&paths, "octo/demo").unwrap();
        // A neighbour that must survive.
        let other_dir = paths.app_dir("someone/else");
        fs::create_dir_all(&other_dir).unwrap();
        fs::write(other_dir.join("keep.txt"), "x").unwrap();

        let new_app = image(900);
        let new_release = Release { tag: "v2.0.0".into(), asset: "Demo-2.0.0-x86_64.AppImage".into(), ..release_for(&new_app, true) };
        let stale = Manifest { version: "v0.1".into(), ..installed.clone() };
        assert!(install(&paths, &prepared(Kind::Update, Some(new_release.clone()), Some(stale)), &serve(&new_app, None, None), false).unwrap_err().contains("has changed"));
        let outcome = install(&paths, &prepared(Kind::Update, Some(new_release), Some(installed.clone())), &serve(&new_app, None, None), false).unwrap();
        assert!(outcome.contains("v2.0.0"), "{outcome}");
        let updated = read_manifest(&paths, "octo/demo").unwrap();
        assert_eq!((updated.version.as_str(), updated.file.as_str(), updated.icon.as_deref()), ("v2.0.0", "Demo-2.0.0-x86_64.AppImage", Some("icon.png")), "the icon is kept");
        assert!(!paths.app_dir("octo/demo").join("Demo-1.2.0-x86_64.AppImage").exists(), "the old file is removed");

        let plan = prepared(Kind::Remove, None, Some(updated.clone()));
        let stale = prepared(Kind::Remove, None, Some(installed));
        assert!(remove(&paths, &stale).unwrap_err().contains("has changed"));
        let outcome = remove(&paths, &plan).unwrap();
        assert!(outcome.contains("Deleted"), "{outcome}");
        assert!(!paths.app_dir("octo/demo").exists() && !paths.launchers.join(launcher_name("octo/demo")).exists());
        assert!(other_dir.join("keep.txt").is_file(), "other apps are untouched");
        assert!(remove(&paths, &plan).is_err(), "nothing left to remove");
        cleanup(&paths);
    }

    #[test]
    fn a_tampered_manifest_is_ignored_and_remove_never_leaves_the_store_folder() {
        let paths = temporary("tamper");
        let app = image(200);
        install(&paths, &prepared(Kind::Install, Some(release_for(&app, false)), None), &serve(&app, None, None), false).unwrap();
        let dir = paths.app_dir("octo/demo");
        let good = fs::read_to_string(dir.join("meta.json")).unwrap();
        for (from, to) in [("Demo-1.2.0-x86_64.AppImage\"", "../../../../etc/passwd\""), ("octo/demo\"", "other/place\""), ("\"nexa-appimage-", "\"nexa-x-")] {
            fs::write(dir.join("meta.json"), good.replacen(from, to, 1)).unwrap();
            assert!(read_manifest(&paths, "octo/demo").is_none(), "{from}");
        }
        fs::write(dir.join("meta.json"), &good).unwrap();
        assert!(read_manifest(&paths, "octo/demo").is_some());
        fs::remove_file(dir.join("Demo-1.2.0-x86_64.AppImage")).unwrap();
        std::os::unix::fs::symlink("/etc/hostname", dir.join("Demo-1.2.0-x86_64.AppImage")).unwrap();
        assert!(read_manifest(&paths, "octo/demo").is_none(), "a symlink is not accepted as the app");
        assert!(read_manifest(&paths, "../../etc").is_none() && read_manifest(&paths, "octo/../demo").is_none());
        // A launcher with another owner's marker is not removed.
        let manifest_ok = Manifest { file: "x.AppImage".into(), ..serde_json::from_str(&good).unwrap() };
        fs::remove_file(dir.join("Demo-1.2.0-x86_64.AppImage")).unwrap();
        fs::write(dir.join("x.AppImage"), image(10)).unwrap();
        fs::write(dir.join("meta.json"), serde_json::to_vec(&manifest_ok).unwrap()).unwrap();
        fs::create_dir_all(&paths.launchers).unwrap();
        let launcher = paths.launchers.join(launcher_name("octo/demo"));
        fs::write(&launcher, "[Desktop Entry]\nName=Mine\nExec=/bin/true\n").unwrap();
        remove(&paths, &prepared(Kind::Remove, None, Some(manifest_ok))).unwrap();
        assert!(launcher.is_file(), "a launcher this module did not write stays");
        cleanup(&paths);
    }

    #[test]
    fn an_installed_app_is_started_and_a_failure_right_after_starting_is_reported() {
        let paths = temporary("open");
        let app = image(100);
        install(&paths, &prepared(Kind::Install, Some(release_for(&app, false)), None), &serve(&app, None, None), false).unwrap();
        let file = paths.app_dir("octo/demo").join("Demo-1.2.0-x86_64.AppImage");
        let marker = paths.base.join("started");
        // Stand-ins for the AppImage: a script that records its start, and one that fails at once.
        fs::write(&file, format!("#!/bin/sh\ntouch {}\nexit 0\n", marker.display())).unwrap();
        open_installed(&paths, "octo/demo").unwrap();
        assert!(marker.exists(), "the app was started");
        fs::write(&file, "#!/bin/sh\nexit 3\n").unwrap();
        let error = open_installed(&paths, "octo/demo").unwrap_err();
        assert!(error.contains("closed right after starting"), "{error}");
        assert!(open_installed(&paths, "nobody/nothing").unwrap_err().contains("does not appear to be installed"));
        cleanup(&paths);
    }

    #[test]
    fn search_text_and_catalogue_packages_are_plain_data() {
        let package = package_from_entry(&entry(), None);
        assert_eq!((package.id.as_str(), package.source, package.kind.as_str(), package.installed), ("octo/demo", Source::Appimage, "application", false));
        assert_eq!(package.categories, vec!["utilities"]);
        assert!(package.icon_url.as_deref().unwrap().starts_with("https://appimage.github.io/database/"));
        let json = serde_json::to_value(&package).unwrap();
        assert_eq!(json["source"], "appimage");
        assert_eq!(launcher_name("Octo/Demo"), launcher_name("octo/demo"));
        assert!(launcher_name("a/b").starts_with("nexa-appimage-") && launcher_name("a/b").ends_with(".desktop"));
    }

    #[test]
    fn the_address_of_a_catalogue_icon_is_encoded_and_cannot_leave_the_database() {
        assert_eq!(icon_address("Foo Bar/icons/a b.png").unwrap(), "https://appimage.github.io/database/Foo%20Bar/icons/a%20b.png");
        for bad in ["", "../x.png", "a/../b.png", "a//b.png", "/abs.png", "a/b.svg", "a/b.png?x=1x/..", "a/\n.png"] {
            let ok = icon_address(bad);
            assert!(ok.as_deref().map(|url| !url.contains("..") && !url.contains("//database") && url.starts_with(ICON_BASE)).unwrap_or(true), "{bad:?} -> {ok:?}");
        }
        assert!(icon_address("../x.png").is_none() && icon_address("a/../b.png").is_none() && icon_address("a/b.svg").is_none());
    }

    #[test]
    #[ignore = "downloads a small real AppImage (appimagetool, a few MB) from GitHub into a temporary folder; nothing is run or installed"]
    fn real_download_is_verified_end_to_end() {
        let paths = temporary("real");
        let release = latest_release("AppImage/appimagetool").expect("the latest release of appimagetool");
        println!("{release:?}");
        assert!(release.size < 40 * 1024 * 1024, "the test is meant for a small file");
        let plan = Prepared { kind: Kind::Install, id: "AppImage/appimagetool".into(), name: "appimagetool".into(), entry: None, release: Some(release.clone()), manifest: None };
        let outcome = install(&paths, &plan, &open_url, false).expect("the real download is accepted");
        println!("{outcome}");
        let manifest = read_manifest(&paths, "AppImage/appimagetool").expect("recorded");
        assert_eq!(manifest.version, release.tag);
        if let Some(expected) = &release.sha256 { assert_eq!(&manifest.sha256, expected); }
        let file = paths.app_dir("AppImage/appimagetool").join(&manifest.file);
        assert_eq!(fs::metadata(&file).unwrap().len(), release.size);
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o755);
        let removal = Prepared { kind: Kind::Remove, id: plan.id.clone(), name: plan.name.clone(), entry: None, release: None, manifest: Some(manifest) };
        remove(&paths, &removal).expect("removal");
        assert!(!paths.app_dir("AppImage/appimagetool").exists());
        cleanup(&paths);
    }

    #[test]
    #[ignore = "reads the real AppImageHub feed and GitHub's API over the network"]
    fn real_catalogue_and_release() {
        // Step by step first, so that a failure says where it happened.
        let (stream, length) = open_url(FEED_URL, Purpose::Catalog).expect("the feed can be opened");
        let data = read_limited(stream, FEED_LIMIT).expect("the feed can be read");
        println!("feed: {} bytes (announced {length:?})", data.len());
        assert!(!parse_feed(&data).expect("the feed can be parsed").is_empty());
        let entries = catalog();
        println!("{} catalogue entries", entries.len());
        assert!(entries.len() > 1000);
        let zettlr = entries.iter().find(|entry| entry.name.eq_ignore_ascii_case("zettlr")).expect("Zettlr is in the catalogue");
        println!("{zettlr:?}");
        let release = latest_release(&zettlr.id).unwrap();
        println!("{release:?}");
        assert!(release.asset.to_ascii_lowercase().ends_with(".appimage"));
        assert!(!search("zettlr").unwrap().is_empty());
        assert!(entries.iter().any(|entry| entry.name == "Beekeeper Studio"), "underscores in names become spaces");
    }
}
