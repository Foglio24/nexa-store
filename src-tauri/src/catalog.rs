//! Read-only AppStream classification and bounded local raster icon lookup.
//!
//! Inputs are package identities, never paths. Desktop-application metadata is
//! the classification authority; merely having an icon does not make an app.
use crate::categories;
use base64::{engine::general_purpose::STANDARD, Engine};
use flate2::read::GzDecoder;
use quick_xml::{events::Event, Reader};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const CACHE_TTL: Duration = Duration::from_secs(600);
const MAX_NAME: usize = 80;
const MAX_SUMMARY: usize = 220;
const MAX_CATEGORIES: usize = 32;
const MAX_METADATA_FILE: u64 = 96 * 1024 * 1024;
const MAX_METADATA_TOTAL: u64 = 192 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 100_000;
const MAX_LINE: usize = 1024 * 1024;
const MAX_ICON_BYTES: u64 = 512 * 1024;
const MAX_ICON_CACHE_BYTES: usize = 16 * 1024 * 1024;
const SIZES: [&str; 5] = ["128x128", "64x64@2", "64x64", "256x256", "48x48"];
static CATALOG: OnceLock<Mutex<Option<Catalog>>> = OnceLock::new();
static IMAGES: OnceLock<Mutex<Option<ImageCache>>> = OnceLock::new();
static FLATPAK_TYPES: OnceLock<Mutex<HashMap<PathBuf, FlatpakTypes>>> = OnceLock::new();

struct FlatpakTypes {
    created: Instant,
    types: HashMap<String, bool>,
}

/// Display metadata of one AppStream desktop application.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppMeta {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub categories: Vec<String>,
}

/// One application that can be listed in the store, from a local AppStream catalog.
#[derive(Clone, Debug)]
pub struct CatalogApp {
    /// `apt` or `flatpak`.
    pub source: &'static str,
    /// APT package name or Flatpak application id.
    pub id: String,
    /// Normalised AppStream id, used to recognise the same app in another source.
    pub app_id: String,
    pub name: String,
    pub summary: String,
    pub categories: Vec<&'static str>,
    pub has_icon: bool,
    pub remote: Option<String>,
    pub scope: Option<String>,
}

#[derive(Default)]
struct Metadata {
    applications: HashSet<String>,
    ids: HashMap<String, Vec<String>>,
    apps: HashMap<String, AppMeta>,
    documents: usize,
    bytes: u64,
}

struct Catalog {
    created: Instant,
    metadata: Metadata,
    icons: HashMap<String, PathBuf>,
    apps: Arc<Vec<CatalogApp>>,
}

struct FlatpakCatalog {
    created: Instant,
    apps: Arc<Vec<CatalogApp>>,
}

struct ImageCache {
    created: Instant,
    bytes: usize,
    values: HashMap<PathBuf, Option<String>>,
}

fn package_name(id: &str) -> Option<&str> {
    let name = id.split_once(':').map_or(id, |(name, arch)| {
        if arch.is_empty() || !arch.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') { "" } else { name }
    });
    (name.len() >= 2 && name.len() <= 255 && name.as_bytes()[0].is_ascii_alphanumeric()
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-".contains(&b))).then_some(name)
}

fn safe_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 255 && !value.starts_with('.') && !value.starts_with('-')
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

fn scalar(value: &str) -> Option<&str> {
    let value = value.trim();
    let value = if value.starts_with('\'') || value.starts_with('"') {
        let quote = value.as_bytes()[0];
        if value.len() < 2 || value.as_bytes().last() != Some(&quote) { return None; }
        &value[1..value.len() - 1]
    } else { value };
    (!value.is_empty() && !value.contains(['\n', '\r', '\\', '\'', '"'])).then_some(value)
}

/// Turn a YAML display value into plain text, or an empty string when it uses a syntax
/// this parser deliberately does not interpret (folded/literal blocks, flow collections).
fn display_text(value: &str, limit: usize) -> String {
    let value = value.trim();
    let text = match value.as_bytes().first() {
        None | Some(b'>') | Some(b'|') | Some(b'&') | Some(b'*') | Some(b'[') | Some(b'{') | Some(b'!') | Some(b'#') => return String::new(),
        Some(b'\'') => {
            if value.len() < 2 || !value.ends_with('\'') { return String::new(); }
            value[1..value.len() - 1].replace("''", "'")
        },
        Some(b'"') => {
            if value.len() < 2 || !value.ends_with('"') { return String::new(); }
            let inner = &value[1..value.len() - 1];
            let mut out = String::new();
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c != '\\' { out.push(c); continue; }
                match chars.next() { Some('"') => out.push('"'), Some('\\') => out.push('\\'), Some('n') | Some('t') => out.push(' '), _ => return String::new() }
            }
            out
        },
        _ => value.to_string(),
    };
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().any(char::is_control) { return String::new(); }
    text.chars().take(limit).collect()
}

/// Per-language values of a translatable DEP-11 field. Italian is used only when the interface is
/// Italian; every other language gets the default (English) text.
#[derive(Default, Clone)]
struct Localized { default: String, italian: String }
impl Localized {
    fn best(&self) -> String { if self.italian.is_empty() || !crate::i18n::italian() { self.default.clone() } else { self.italian.clone() } }
}

#[derive(Default)]
struct Document {
    kind: String,
    package: String,
    id: String,
    name: Localized,
    summary: Localized,
    categories: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section { Other, Name, Summary, Categories }

fn finish_document(metadata: &mut Metadata, document: &Document) {
    metadata.documents += 1;
    if document.kind != "desktop-application" { return; }
    if let Some(package) = package_name(&document.package) {
        metadata.applications.insert(package.to_string());
        if safe_name(&document.id) {
            let values = metadata.ids.entry(package.to_string()).or_default();
            if values.len() < 16 && !values.iter().any(|value| value == &document.id) { values.push(document.id.clone()); }
        }
        if metadata.apps.len() < MAX_DOCUMENTS && !metadata.apps.contains_key(package) {
            metadata.apps.insert(package.to_string(), AppMeta {
                id: if safe_name(&document.id) { document.id.clone() } else { String::new() },
                name: document.name.best(),
                summary: document.summary.best(),
                categories: document.categories.clone(),
            });
        }
    }
}

/// Classification fields (`Type`, `Package`, `ID`) are read only at column zero, so nested
/// metadata, descriptions, add-ons, codecs and fonts cannot supply them. Display fields are
/// read only from their own top-level block: `Name:`/`Summary:` (keys `C` and `it`) and the
/// `Categories:` list.
fn parse_metadata(reader: impl BufRead, byte_limit: u64, document_limit: usize) -> io::Result<Metadata> {
    let mut reader = reader;
    let mut result = Metadata::default();
    let mut document = Document::default();
    let mut section = Section::Other;
    let mut line = String::new();
    let has_content = |document: &Document| !document.kind.is_empty() || !document.package.is_empty() || !document.id.is_empty();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 { break; }
        result.bytes += read as u64;
        if result.bytes > byte_limit || line.len() > MAX_LINE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "AppStream metadata limit exceeded"));
        }
        let text = line.trim_end_matches(['\r', '\n']);
        if matches!(text, "---" | "...") {
            if has_content(&document) {
                finish_document(&mut result, &document);
                if result.documents >= document_limit { return Ok(result); }
            }
            document = Document::default();
            section = Section::Other;
        } else if let Some(item) = text.strip_prefix("- ") {
            if section == Section::Categories && document.categories.len() < MAX_CATEGORIES {
                let value = display_text(item, 40);
                if !value.is_empty() { document.categories.push(value); }
            }
        } else if let Some(nested) = text.strip_prefix("  ") {
            if !nested.starts_with(' ') && matches!(section, Section::Name | Section::Summary) {
                if let Some((language, value)) = nested.split_once(':') {
                    let limit = if section == Section::Name { MAX_NAME } else { MAX_SUMMARY };
                    let target = if section == Section::Name { &mut document.name } else { &mut document.summary };
                    match language { "C" => target.default = display_text(value, limit), "it" => target.italian = display_text(value, limit), _ => {} }
                }
            }
        } else if !text.starts_with(' ') {
            if let Some((key, value)) = text.split_once(':') {
                section = match key { "Name" => Section::Name, "Summary" => Section::Summary, "Categories" => Section::Categories, _ => Section::Other };
                let destination = match key { "Type" => &mut document.kind, "Package" => &mut document.package, "ID" => &mut document.id, _ => continue };
                *destination = scalar(value).unwrap_or("").to_string();
            }
        }
    }
    if has_content(&document) { finish_document(&mut result, &document); }
    Ok(result)
}

fn sorted_children(directory: &Path, limit: usize) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(directory).into_iter().flatten().take(limit)
        .filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
    paths.sort(); paths
}

fn within(path: &Path, root: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let path = path.canonicalize().ok()?;
    path.starts_with(root).then_some(path)
}

/// For a `.desktop` file that is a visible application (Type=Application, with an Exec line, and
/// neither NoDisplay nor Hidden), the icon name it declares (possibly empty); otherwise `None`.
fn desktop_entry_icon(text: &str) -> Option<String> {
    let (mut group, mut application, mut hidden, mut executable) = (false, false, false, false);
    let mut icon = String::new();
    for line in text.lines() {
        if line.starts_with('[') { group = line == "[Desktop Entry]"; }
        if !group { continue; }
        if line == "Type=Application" { application = true; }
        if matches!(line, "NoDisplay=true" | "Hidden=true") { hidden = true; }
        if line.starts_with("Exec=") && line.len() > 5 { executable = true; }
        if let Some(value) = line.strip_prefix("Icon=") { if safe_name(value) { icon = value.to_string(); } }
    }
    (application && executable && !hidden).then_some(icon)
}

/// The visible launcher snapd exports for an installed snap (`<name>_<app>.desktop`); `None` for
/// snaps used from a terminal. `name` is validated, never a path.
pub fn snap_desktop_file(name: &str) -> Option<PathBuf> {
    if package_name(name) != Some(name) || name.contains(['+', '.']) { return None; }
    let root = Path::new("/var/lib/snapd/desktop/applications");
    let prefix = format!("{name}_");
    for desktop in sorted_children(root, 4096) {
        let Some(file) = desktop.file_name().and_then(|file| file.to_str()) else { continue; };
        if !file.starts_with(&prefix) || !file.ends_with(".desktop") { continue; }
        let Some(path) = within(&desktop, root) else { continue; };
        if fs::metadata(&path).map(|m| m.len() > 64 * 1024).unwrap_or(true) { continue; }
        if fs::read_to_string(&path).ok().and_then(|text| desktop_entry_icon(&text)).is_some() { return Some(path); }
    }
    None
}

/// The visible launcher (`/usr/share/applications/*.desktop`) installed by an APT package, found in
/// dpkg's file list. Used to open an installed app; `package` is validated, never a path.
pub fn apt_desktop_file(package: &str) -> Option<PathBuf> {
    let package = package_name(package)?;
    let root = Path::new("/usr/share/applications");
    let info = Path::new("/var/lib/dpkg/info");
    for list in [format!("{package}.list"), format!("{package}:amd64.list"), format!("{package}:arm64.list")] {
        let Some(list) = within(&info.join(list), info) else { continue; };
        let Ok(file) = fs::File::open(list) else { continue; };
        for line in BufReader::new(file.take(4 * 1024 * 1024)).lines().map_while(|line| line.ok()) {
            if !line.starts_with("/usr/share/applications/") || !line.ends_with(".desktop") { continue; }
            let Some(path) = within(Path::new(&line), root) else { continue; };
            if fs::metadata(&path).map(|m| m.len() > 64 * 1024).unwrap_or(true) { continue; }
            if fs::read_to_string(&path).ok().and_then(|text| desktop_entry_icon(&text)).is_some() { return Some(path); }
        }
    }
    None
}

// Installed third-party packages may have no distribution AppStream entry.
// A visible Desktop Entry owned by a dpkg package is a second positive signal.
fn installed_desktops(metadata: &mut Metadata) {
    let root = Path::new("/usr/share/applications");
    let mut desktops = HashMap::new();
    for path in sorted_children(root, 4096) {
        if path.extension().and_then(|value| value.to_str()) != Some("desktop") { continue; }
        let Some(path) = within(&path, root) else { continue; };
        if fs::metadata(&path).map(|value| value.len() > 64 * 1024).unwrap_or(true) { continue; }
        let Ok(text) = fs::read_to_string(&path) else { continue; };
        if let Some(icon) = desktop_entry_icon(&text) {
            desktops.insert(path.to_string_lossy().into_owned(), icon);
        }
    }
    let info_root = Path::new("/var/lib/dpkg/info");
    let mut budget = 64 * 1024 * 1024u64;
    for path in sorted_children(info_root, 65_536) {
        if budget == 0 { break; }
        let Some(name) = path.file_name().and_then(|value| value.to_str()).and_then(|value| value.strip_suffix(".list")) else { continue; };
        let Some(package) = package_name(name) else { continue; };
        let Some(path) = within(&path, info_root) else { continue; };
        let Ok(file) = fs::File::open(path) else { continue; };
        let mut reader = BufReader::new(file.take(budget.min(4 * 1024 * 1024)));
        let mut line = String::new();
        loop {
            line.clear();
            let Ok(count) = reader.read_line(&mut line) else { break; };
            if count == 0 { break; }
            budget = budget.saturating_sub(count as u64);
            if let Some(icon) = desktops.get(line.trim_end_matches(['\r', '\n'])) {
                metadata.applications.insert(package.to_string());
                if !icon.is_empty() { metadata.ids.entry(package.to_string()).or_default().push(icon.clone()); }
                break;
            }
        }
    }
}

fn load_catalog() -> Catalog {
    let mut metadata = Metadata::default();
    let directory = Path::new("/var/lib/swcatalog/yaml");
    for path in sorted_children(directory, 128) {
        if metadata.bytes >= MAX_METADATA_TOTAL || metadata.documents >= MAX_DOCUMENTS { break; }
        if !path.file_name().and_then(|name| name.to_str()).map(|name| name.ends_with(".yml.gz")).unwrap_or(false) { continue; }
        // Ubuntu intentionally links swcatalog metadata into APT's root-owned lists.
        let canonical = within(&path, directory).or_else(|| within(&path, Path::new("/var/lib/apt/lists")));
        let Some(path) = canonical else { continue; };
        let Ok(file) = fs::File::open(path) else { continue; };
        let limit = MAX_METADATA_FILE.min(MAX_METADATA_TOTAL.saturating_sub(metadata.bytes));
        let decoder = GzDecoder::new(file).take(limit + 1);
        if let Ok(parsed) = parse_metadata(BufReader::new(decoder), limit, MAX_DOCUMENTS - metadata.documents) {
            metadata.bytes += parsed.bytes;
            metadata.documents += parsed.documents;
            metadata.applications.extend(parsed.applications);
            for (package, ids) in parsed.ids { metadata.ids.entry(package).or_default().extend(ids); }
            for (package, meta) in parsed.apps { metadata.apps.entry(package).or_insert(meta); }
        } else {
            // Charge failed/oversized files against the total decompression budget.
            metadata.bytes += limit;
        }
    }
    installed_desktops(&mut metadata);
    let mut icons = HashMap::new();
    let icon_root = Path::new("/var/lib/swcatalog/icons");
    for size in SIZES {
        for origin in sorted_children(icon_root, 128) {
            if within(&origin, icon_root).is_none() { continue; }
            for path in sorted_children(&origin.join(size), 30_000) {
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue; };
                let Some((package, _)) = name.split_once('_') else { continue; };
                if metadata.applications.contains(package) && !icons.contains_key(package) && raster_extension(&path) {
                    if let Some(canonical) = within(&path, icon_root) { icons.insert(package.to_string(), canonical); }
                }
            }
        }
    }
    let mut apps: Vec<CatalogApp> = metadata.apps.iter().map(|(package, meta)| {
        let mut names = metadata.ids.get(package).cloned().unwrap_or_default();
        names.push(package.clone());
        CatalogApp {
            source: "apt",
            id: package.clone(),
            app_id: categories::id_key(&meta.id),
            name: if meta.name.is_empty() { package.clone() } else { meta.name.clone() },
            summary: meta.summary.clone(),
            categories: categories::from_freedesktop_list(&meta.categories),
            has_icon: icons.contains_key(package) || theme_has_icon(&names),
            remote: None,
            scope: None,
        }
    }).collect();
    apps.sort_by(|a, b| a.id.cmp(&b.id));
    Catalog { created: Instant::now(), metadata, icons, apps: Arc::new(apps) }
}

fn with_catalog<T>(read: impl FnOnce(&Catalog) -> T) -> T {
    let mut cache = CATALOG.get_or_init(|| Mutex::new(None)).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.as_ref().map(|catalog| catalog.created.elapsed() >= CACHE_TTL).unwrap_or(true) {
        *cache = Some(load_catalog());
    }
    read(cache.as_ref().expect("catalog initialized"))
}

pub fn apt_application_ids() -> HashSet<String> {
    with_catalog(|catalog| catalog.metadata.applications.clone())
}

pub fn apt_is_application(id: &str) -> bool {
    package_name(id).map(|id| with_catalog(|catalog| catalog.metadata.applications.contains(id))).unwrap_or(false)
}

fn raster_extension(path: &Path) -> bool {
    matches!(path.extension().and_then(|name| name.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("png" | "jpg" | "jpeg" | "webp"))
}

fn raster_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { Some("image/png") }
    else if bytes.starts_with(b"\xff\xd8\xff") { Some("image/jpeg") }
    else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") { Some("image/webp") }
    else { None }
}

fn image_data(path: &Path, root: &Path) -> Option<String> {
    if !raster_extension(path) { return None; }
    let path = within(path, root)?;
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_ICON_BYTES { return None; }
    let mut cache = IMAGES.get_or_init(|| Mutex::new(None)).lock().ok()?;
    if cache.as_ref().map(|images| images.created.elapsed() >= CACHE_TTL).unwrap_or(true) {
        *cache = Some(ImageCache { created: Instant::now(), bytes: 0, values: HashMap::new() });
    }
    let images = cache.as_mut()?;
    if let Some(value) = images.values.get(&path) { return value.clone(); }
    let mut bytes = Vec::new();
    fs::File::open(&path).ok()?.take(MAX_ICON_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_ICON_BYTES { return None; }
    let value = raster_mime(&bytes).map(|mime| format!("data:{mime};base64,{}", STANDARD.encode(bytes)));
    let length = value.as_ref().map(String::len).unwrap_or(0);
    if images.values.len() >= 512 || images.bytes + length > MAX_ICON_CACHE_BYTES {
        images.values.clear(); images.bytes = 0;
    }
    images.bytes += length;
    images.values.insert(path, value.clone());
    value
}

fn theme_icon(names: &[String]) -> Option<String> {
    for root in [Path::new("/usr/share/icons/hicolor"), Path::new("/usr/share/pixmaps")] {
        for name in names.iter().filter(|name| safe_name(name)) {
            let name = name.strip_suffix(".desktop").unwrap_or(name);
            for extension in ["png", "webp", "jpg", "jpeg"] {
                if root.ends_with("pixmaps") {
                    if let Some(data) = image_data(&root.join(format!("{name}.{extension}")), root) { return Some(data); }
                } else {
                    for size in SIZES {
                        if let Some(data) = image_data(&root.join(size).join("apps").join(format!("{name}.{extension}")), root) { return Some(data); }
                    }
                }
            }
        }
    }
    None
}

/// Same lookup as `theme_icon`, but only checks that a raster icon exists (no read, no encoding).
fn theme_has_icon(names: &[String]) -> bool {
    for root in [Path::new("/usr/share/icons/hicolor"), Path::new("/usr/share/pixmaps")] {
        for name in names.iter().filter(|name| safe_name(name)) {
            let name = name.strip_suffix(".desktop").unwrap_or(name);
            for extension in ["png", "webp", "jpg", "jpeg"] {
                if root.ends_with("pixmaps") {
                    if root.join(format!("{name}.{extension}")).is_file() { return true; }
                } else {
                    for size in SIZES {
                        if root.join(size).join("apps").join(format!("{name}.{extension}")).is_file() { return true; }
                    }
                }
            }
        }
    }
    false
}

fn apt_icon(id: &str) -> Option<String> {
    let id = package_name(id)?;
    let (path, mut names) = with_catalog(|catalog| (catalog.icons.get(id).cloned(), catalog.metadata.ids.get(id).cloned().unwrap_or_default()));
    if let Some(path) = path {
        if let Some(data) = image_data(&path, Path::new("/var/lib/swcatalog/icons")) { return Some(data); }
    }
    names.push(id.to_string());
    theme_icon(&names)
}

fn user_data_home() -> Option<PathBuf> {
    if let Some(value) = env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(value);
        if path.is_absolute() { return Some(path); }
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
}

fn flatpak_roots(scope: Option<&str>) -> Option<Vec<PathBuf>> {
    match scope {
        Some("user") => Some(vec![user_data_home()?.join("flatpak/appstream")]),
        Some("system") => Some(vec![PathBuf::from("/var/lib/flatpak/appstream")]),
        None => {
            let mut roots = Vec::new();
            if let Some(home) = user_data_home() { roots.push(home.join("flatpak/appstream")); }
            roots.push(PathBuf::from("/var/lib/flatpak/appstream")); Some(roots)
        },
        _ => None,
    }
}

fn flatpak_app_id(id: &str) -> Option<&str> {
    let id = if id.starts_with("app/") {
        let mut parts = id.split('/'); parts.next();
        let app = parts.next()?;
        if parts.count() != 2 { return None; }
        app
    } else { id };
    (safe_name(id) && id.contains('.')).then_some(id)
}

/// One `<component>` of a Flatpak AppStream catalog, reduced to what the store needs.
#[derive(Default, Clone)]
struct FlatpakComponent {
    id: String,
    application: bool,
    name: Localized,
    summary: Localized,
    categories: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field { Id, Name(Lang), Summary(Lang), Category }
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang { Default, Italian, Other }

fn xml_language(element: &quick_xml::events::BytesStart<'_>) -> Lang {
    for attribute in element.attributes().flatten() {
        if attribute.key.as_ref() == "xml:lang" {
            let value = attribute.value.as_ref();
            return if value == "it" || value.starts_with("it_") || value.starts_with("it-") { Lang::Italian } else { Lang::Other };
        }
    }
    Lang::Default
}

fn xml_entity(name: &str) -> Option<char> {
    match name { "amp" => Some('&'), "lt" => Some('<'), "gt" => Some('>'), "quot" => Some('"'), "apos" => Some('\''), _ => None }
}

fn collapse(text: &str, limit: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().any(char::is_control) { return String::new(); }
    joined.chars().take(limit).collect()
}

/// Streams a Flatpak `appstream.xml`. Only direct children of `<component>` are read, so
/// nested ids and the `<developer><name>` element can never override the app's own fields.
/// Entity references arrive as separate events in quick-xml 0.42, hence no text trimming.
fn parse_flatpak_components(input: impl BufRead) -> io::Result<Vec<FlatpakComponent>> {
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut component: Option<(usize, FlatpakComponent)> = None;
    let mut categories_depth: Option<usize> = None;
    let mut field: Option<(usize, Field)> = None;
    let mut text = String::new();
    loop {
        let event = reader.read_event_into(&mut buffer).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if reader.buffer_position() > MAX_METADATA_FILE { return Err(io::Error::new(io::ErrorKind::InvalidData, "Flatpak metadata too large")); }
        match event {
            Event::Start(element) => {
                depth += 1;
                let name = element.name();
                let name = name.as_ref();
                match component.as_mut() {
                    None if name == "component" => {
                        let mut application = false;
                        for attribute in element.attributes().flatten() {
                            if attribute.key.as_ref() == "type" { application = attribute.value.as_ref() == "desktop-application"; }
                        }
                        component = Some((depth, FlatpakComponent { application, ..Default::default() }));
                    },
                    Some((component_depth, _)) if field.is_none() => {
                        let component_depth = *component_depth;
                        let kind = if depth == component_depth + 1 {
                            match name {
                                "id" => Some(Field::Id),
                                "name" => Some(Field::Name(xml_language(&element))),
                                "summary" => Some(Field::Summary(xml_language(&element))),
                                "categories" => { categories_depth = Some(depth); None },
                                _ => None,
                            }
                        } else if depth == component_depth + 2 && categories_depth == Some(depth - 1) && name == "category" {
                            Some(Field::Category)
                        } else { None };
                        if let Some(kind) = kind { field = Some((depth, kind)); text.clear(); }
                    },
                    _ => {},
                }
            },
            Event::Text(value) if field.is_some() => {
                if text.len() < 1024 { text.push_str(value.as_ref()); }
            },
            Event::GeneralRef(reference) if field.is_some() => {
                let name: &str = reference.as_ref();
                let resolved = if reference.is_char_ref() { reference.resolve_char_ref().ok().flatten() } else { xml_entity(name) };
                if let Some(character) = resolved { if text.len() < 1024 { text.push(character); } }
            },
            Event::End(element) => {
                if let (Some((field_depth, kind)), Some((_, data))) = (field, component.as_mut()) {
                    if field_depth == depth {
                        match kind {
                            Field::Id => data.id = collapse(&text, 255),
                            Field::Name(Lang::Default) => data.name.default = collapse(&text, MAX_NAME),
                            Field::Name(Lang::Italian) => data.name.italian = collapse(&text, MAX_NAME),
                            Field::Summary(Lang::Default) => data.summary.default = collapse(&text, MAX_SUMMARY),
                            Field::Summary(Lang::Italian) => data.summary.italian = collapse(&text, MAX_SUMMARY),
                            Field::Category => {
                                let value = collapse(&text, 40);
                                if !value.is_empty() && data.categories.len() < MAX_CATEGORIES { data.categories.push(value); }
                            },
                            Field::Name(Lang::Other) | Field::Summary(Lang::Other) => {},
                        }
                        field = None;
                    }
                }
                if categories_depth == Some(depth) { categories_depth = None; }
                if element.name().as_ref() == "component" && component.as_ref().map(|(d, _)| *d) == Some(depth) {
                    if let Some((_, data)) = component.take() {
                        if flatpak_app_id(&data.id).is_some() { result.push(data); }
                    }
                    if result.len() >= MAX_DOCUMENTS { break; }
                }
                depth = depth.saturating_sub(1);
            },
            Event::Eof => break,
            _ => {},
        }
        buffer.clear();
    }
    Ok(result)
}

fn parse_flatpak_types(input: impl BufRead) -> io::Result<HashMap<String, bool>> {
    Ok(parse_flatpak_components(input)?.into_iter().map(|component| (component.id, component.application)).collect())
}

/// Some(false) identifies an add-on/font/codec rather than a desktop app;
/// None means the configured remote has no usable local classification metadata.
pub fn flatpak_is_application(id: &str, scope: Option<&str>, remote: Option<&str>) -> Option<bool> {
    let id = flatpak_app_id(id)?;
    if remote.map(|name| !safe_name(name)).unwrap_or(false) { return None; }
    let mut cache = FLATPAK_TYPES.get_or_init(|| Mutex::new(HashMap::new())).lock().ok()?;
    cache.retain(|_, value| value.created.elapsed() < CACHE_TTL);
    for root in flatpak_roots(scope)? {
        let remotes = remote.map(|name| vec![root.join(name)]).unwrap_or_else(|| sorted_children(&root, 64));
        for directory in remotes {
            for arch in sorted_children(&directory, 16) {
                let xml = arch.join("active/appstream.xml.gz");
                let Some(path) = within(&xml, &root) else { continue; };
                if !cache.contains_key(&path) {
                    if cache.len() >= 32 { cache.clear(); }
                    let file = fs::File::open(&path).ok()?;
                    let decoder = GzDecoder::new(file).take(MAX_METADATA_FILE + 1);
                    let types = parse_flatpak_types(BufReader::new(decoder)).unwrap_or_default();
                    cache.insert(path.clone(), FlatpakTypes { created: Instant::now(), types });
                }
                if let Some(value) = cache.get(&path).and_then(|metadata| metadata.types.get(id)) { return Some(*value); }
            }
        }
    }
    None
}

static FLATPAK_APPS: OnceLock<Mutex<Option<FlatpakCatalog>>> = OnceLock::new();

/// Display name, summary and categories of an APT desktop application, if AppStream knows it.
pub fn apt_meta(package: &str) -> Option<AppMeta> {
    let package = package_name(package)?;
    with_catalog(|catalog| catalog.metadata.apps.get(package).cloned())
}

/// Every APT desktop application in the local AppStream catalog.
pub fn apt_apps() -> Arc<Vec<CatalogApp>> {
    with_catalog(|catalog| catalog.apps.clone())
}

fn load_flatpak_apps() -> Vec<CatalogApp> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let Some(roots) = flatpak_roots(None) else { return result; };
    for root in roots {
        let scope = if root.starts_with("/var/lib/flatpak") { "system" } else { "user" };
        for remote_directory in sorted_children(&root, 64) {
            let Some(remote) = remote_directory.file_name().and_then(|name| name.to_str()).filter(|name| safe_name(name)).map(str::to_string) else { continue; };
            if within(&remote_directory, &root).is_none() { continue; }
            for arch in sorted_children(&remote_directory, 16) {
                if arch.file_name().and_then(|name| name.to_str()) != Some(env::consts::ARCH) { continue; }
                let Some(path) = within(&arch.join("active/appstream.xml.gz"), &root) else { continue; };
                let Ok(file) = fs::File::open(&path) else { continue; };
                let decoder = GzDecoder::new(file).take(MAX_METADATA_FILE + 1);
                let Ok(components) = parse_flatpak_components(BufReader::new(decoder)) else { continue; };
                let icon_files: HashSet<String> = sorted_children(&arch.join("active/icons/128x128"), 30_000).into_iter()
                    .filter_map(|path| path.file_name().and_then(|name| name.to_str()).map(str::to_string)).collect();
                for component in components {
                    // The user installation is listed first and wins over the system one.
                    if !component.application || !seen.insert(component.id.clone()) { continue; }
                    let name = component.name.best();
                    result.push(CatalogApp {
                        source: "flatpak",
                        app_id: categories::id_key(&component.id),
                        has_icon: icon_files.contains(&format!("{}.png", component.id)),
                        name: if name.is_empty() { component.id.clone() } else { name },
                        summary: component.summary.best(),
                        categories: categories::from_freedesktop_list(&component.categories),
                        id: component.id,
                        remote: Some(remote.clone()),
                        scope: Some(scope.into()),
                    });
                }
            }
        }
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    result
}

/// Every Flatpak desktop application of the configured remotes, from their local AppStream data.
pub fn flatpak_apps() -> Arc<Vec<CatalogApp>> {
    let mut cache = FLATPAK_APPS.get_or_init(|| Mutex::new(None)).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.as_ref().map(|catalog| catalog.created.elapsed() >= CACHE_TTL).unwrap_or(true) {
        *cache = Some(FlatpakCatalog { created: Instant::now(), apps: Arc::new(load_flatpak_apps()) });
    }
    cache.as_ref().expect("flatpak catalog initialized").apps.clone()
}

/// Load both catalogs once so the first visit to the store does not wait for them.
pub fn warm_up() {
    let _ = apt_apps();
    let _ = flatpak_apps();
}

/// Description of the component `wanted` in a Flatpak AppStream stream: paragraphs separated by
/// blank lines, list items as bullets. Italian is used when the interface is Italian and the catalog has it.
fn parse_flatpak_description(input: impl BufRead, wanted: &str) -> io::Result<Option<String>> {
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut depth = 0usize;
    let mut component: Option<(usize, String)> = None;
    let mut description_depth: Option<usize> = None;
    // The language usually sits on <description> itself, one element per language.
    let mut description_lang = Lang::Default;
    let mut block: Option<(usize, Lang, bool)> = None;
    let mut id_depth: Option<usize> = None;
    let mut id = String::new();
    let (mut default_blocks, mut italian_blocks): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let mut text = String::new();
    loop {
        let event = reader.read_event_into(&mut buffer).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if reader.buffer_position() > MAX_METADATA_FILE { return Err(io::Error::new(io::ErrorKind::InvalidData, "Flatpak metadata too large")); }
        match event {
            Event::Start(element) => {
                depth += 1;
                let name = element.name();
                let name = name.as_ref();
                if component.is_none() {
                    if name == "component" { component = Some((depth, String::new())); id.clear(); default_blocks.clear(); italian_blocks.clear(); }
                } else if let Some((component_depth, _)) = &component {
                    if depth == component_depth + 1 && name == "id" { id_depth = Some(depth); text.clear(); }
                    else if depth == component_depth + 1 && name == "description" { description_depth = Some(depth); description_lang = xml_language(&element); }
                    else if description_depth.map(|d| depth == d + 1).unwrap_or(false) && name == "p" {
                        let own = xml_language(&element);
                        block = Some((depth, if own == Lang::Default { description_lang } else { own }, false)); text.clear();
                    } else if description_depth.map(|d| depth == d + 2).unwrap_or(false) && name == "li" {
                        let own = xml_language(&element);
                        block = Some((depth, if own == Lang::Default { description_lang } else { own }, true)); text.clear();
                    }
                }
            },
            Event::Text(value) if block.is_some() || id_depth.is_some() => { if text.len() < 4096 { text.push_str(value.as_ref()); } },
            Event::GeneralRef(reference) if block.is_some() || id_depth.is_some() => {
                let name: &str = reference.as_ref();
                let resolved = if reference.is_char_ref() { reference.resolve_char_ref().ok().flatten() } else { xml_entity(name) };
                if let Some(character) = resolved { if text.len() < 4096 { text.push(character); } }
            },
            Event::End(element) => {
                if id_depth == Some(depth) { id = collapse(&text, 255); id_depth = None; }
                if let Some((block_depth, language, bullet)) = block {
                    if block_depth == depth {
                        let value = collapse(&text, 1200);
                        if !value.is_empty() {
                            let line = if bullet { format!("• {value}") } else { value };
                            match language { Lang::Default => default_blocks.push(line), Lang::Italian => italian_blocks.push(line), Lang::Other => {} }
                        }
                        block = None;
                    }
                }
                if description_depth == Some(depth) { description_depth = None; }
                if element.name().as_ref() == "component" && component.as_ref().map(|(d, _)| *d) == Some(depth) {
                    component = None;
                    if id == wanted {
                        let blocks = if italian_blocks.is_empty() || !crate::i18n::italian() { &default_blocks } else { &italian_blocks };
                        let mut result = String::new();
                        for (index, line) in blocks.iter().enumerate() {
                            if index > 0 { result.push_str(if line.starts_with("• ") && blocks[index - 1].starts_with("• ") { "\n" } else { "\n\n" }); }
                            result.push_str(line);
                        }
                        return Ok(Some(result.chars().take(6000).collect()));
                    }
                }
                depth = depth.saturating_sub(1);
            },
            Event::Eof => break,
            _ => {},
        }
        buffer.clear();
    }
    Ok(None)
}

/// AppStream description of one Flatpak app, read on demand from the local catalog of its remote.
pub fn flatpak_description(id: &str, scope: Option<&str>, remote: Option<&str>) -> Option<String> {
    let id = flatpak_app_id(id)?;
    if remote.map(|name| !safe_name(name)).unwrap_or(false) { return None; }
    for root in flatpak_roots(scope)? {
        let remotes = remote.map(|name| vec![root.join(name)]).unwrap_or_else(|| sorted_children(&root, 64));
        for directory in remotes {
            if within(&directory, &root).is_none() { continue; }
            for arch in sorted_children(&directory, 16) {
                if arch.file_name().and_then(|name| name.to_str()) != Some(env::consts::ARCH) { continue; }
                let Some(path) = within(&arch.join("active/appstream.xml.gz"), &root) else { continue; };
                let Ok(file) = fs::File::open(&path) else { continue; };
                let decoder = GzDecoder::new(file).take(MAX_METADATA_FILE + 1);
                if let Ok(Some(text)) = parse_flatpak_description(BufReader::new(decoder), id) { if !text.is_empty() { return Some(text); } }
            }
        }
    }
    None
}

fn flatpak_icon(id: &str, scope: Option<&str>, remote: Option<&str>) -> Option<String> {
    let id = flatpak_app_id(id)?;
    if remote.map(|name| !safe_name(name)).unwrap_or(false) { return None; }
    for root in flatpak_roots(scope)? {
        let remotes = remote.map(|name| vec![root.join(name)]).unwrap_or_else(|| sorted_children(&root, 64));
        for directory in remotes {
            if within(&directory, &root).is_none() { continue; }
            for arch in sorted_children(&directory, 16) {
                if !arch.file_name().and_then(|name| name.to_str()).map(safe_name).unwrap_or(false) { continue; }
                for size in SIZES {
                    for extension in ["png", "webp", "jpg", "jpeg"] {
                        let path = arch.join("active/icons").join(size).join(format!("{id}.{extension}"));
                        if let Some(data) = image_data(&path, &root) { return Some(data); }
                    }
                }
            }
        }
    }
    theme_icon(&[id.to_string()])
}

fn snap_icon(id: &str) -> Option<String> {
    if package_name(id) != Some(id) || id.contains(['+', '.']) { return None; }
    let root = Path::new("/snap").join(id);
    for extension in ["png", "webp", "jpg", "jpeg"] {
        if let Some(data) = image_data(&root.join("current/meta/gui").join(format!("icon.{extension}")), &root) { return Some(data); }
    }
    let desktop_root = Path::new("/var/lib/snapd/desktop/applications");
    let prefix = format!("{id}_");
    for desktop in sorted_children(desktop_root, 4096) {
        let Some(name) = desktop.file_name().and_then(|name| name.to_str()) else { continue; };
        if !name.starts_with(&prefix) || !name.ends_with(".desktop") { continue; }
        let Some(path) = within(&desktop, desktop_root) else { continue; };
        let Ok(file) = fs::File::open(path) else { continue; };
        let mut text = String::new();
        if file.take(64 * 1024).read_to_string(&mut text).is_err() { continue; }
        let mut main_group = false;
        for line in text.lines() {
            if line.starts_with('[') { main_group = line == "[Desktop Entry]"; }
            if !main_group { continue; }
            if let Some(icon) = line.strip_prefix("Icon=") {
                if icon.starts_with('/') {
                    if let Some(data) = image_data(Path::new(icon), &root) { return Some(data); }
                } else if safe_name(icon) {
                    if let Some(data) = theme_icon(&[icon.to_string()]) { return Some(data); }
                }
            }
        }
    }
    theme_icon(&[id.to_string()])
}

/// A raster image inside `root` as a data URI (same limits and checks as the catalog icons).
pub fn image_in(path: &Path, root: &Path) -> Option<String> { image_data(path, root) }

/// Return a local image as a data URI. No network request or shell is involved.
pub fn local_icon(source: &str, id: &str, scope: Option<&str>, remote: Option<&str>) -> Option<String> {
    match source { "apt" => apt_icon(id), "flatpak" => flatpak_icon(id, scope, remote), "snap" => snap_icon(id), _ => None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_top_level_desktop_application_documents_classify_packages() {
        let input = b"---\nFile: DEP-11\n---\nType: desktop-application\nID: org.gimp.GIMP\nPackage: gimp\n---\nType: addon\nPackage: gimp-data\nIcon:\n  cached: gimp-data.png\nDescription:\n  Type: desktop-application\n  Package: fake-app\n---\nType: codec\nPackage: gstreamer-codec\n---\nType: 'desktop-application'\nPackage: \"vlc\"\nID: vlc.desktop\n";
        let parsed = parse_metadata(BufReader::new(&input[..]), 4096, 100).unwrap();
        assert_eq!(parsed.applications, HashSet::from(["gimp".to_string(), "vlc".to_string()]));
        assert_eq!(parsed.ids["gimp"], vec!["org.gimp.GIMP"]);
        assert!(!parsed.applications.contains("gimp-data"));
        assert!(!parsed.applications.contains("fake-app"));
    }

    #[test]
    fn metadata_size_and_identity_limits_reject_untrusted_inputs() {
        assert!(parse_metadata(BufReader::new(&b"Type: desktop-application\nPackage: gimp\n"[..]), 12, 100).is_err());
        for invalid in ["../secret", "gimp/path", "--option", "gimp:amd64:extra", "gimp\n"] { assert!(package_name(invalid).is_none()); }
        assert_eq!(package_name("gimp:amd64"), Some("gimp"));
        for invalid in ["../icon", "org.foo/bar", ".hidden", "--option"] { assert!(!safe_name(invalid)); }
        assert!(raster_mime(b"<svg onload=\"bad()\"></svg>").is_none());
        assert!(raster_mime(b"\x89PNG\r\n\x1a\n").is_some());
    }

    #[test]
    fn flatpak_xml_distinguishes_apps_from_addons_and_ignores_nested_ids() {
        let xml = br#"<components><component type="desktop-application"><id>org.gimp.GIMP</id><provides><id>org.fake.Nested</id></provides></component><component type="addon"><id>org.gimp.GIMP.Plugin</id></component><component type="font"><id>org.example.Font</id></component></components>"#;
        let parsed = parse_flatpak_types(BufReader::new(&xml[..])).unwrap();
        assert_eq!(parsed.get("org.gimp.GIMP"), Some(&true));
        assert_eq!(parsed.get("org.gimp.GIMP.Plugin"), Some(&false));
        assert_eq!(parsed.get("org.example.Font"), Some(&false));
        assert!(!parsed.contains_key("org.fake.Nested"));
    }

    #[test]
    fn dep11_display_fields_prefer_italian_only_in_an_italian_interface_and_ignore_nested_blocks() {
        use crate::i18n::{with_language, Lang};
        let input = b"---\nType: desktop-application\nID: org.gimp.GIMP\nPackage: gimp\nName:\n  C: GNU Image Manipulation Program\n  it: Programma di ritocco\n  de: Bildbearbeitung\nSummary:\n  C: \"Create \\\"images\\\"\"\nCategories:\n- Graphics\n- 2DGraphics\nKeywords:\n  C:\n  - Photoshop\nDescription:\n  C: >-\n  Name: finto\n---\nType: desktop-application\nID: tom.desktop\nPackage: tom\nName:\n  C: 'Tom''s App'\nSummary:\n  C: >-\n    folded text is not interpreted\n---\nType: addon\nPackage: gimp-data\nName:\n  C: Not an app\n";
        let english = with_language(Lang::En, || parse_metadata(BufReader::new(&input[..]), 8192, 100).unwrap());
        assert_eq!(english.apps["gimp"].name, "GNU Image Manipulation Program", "other languages get the default text");
        let parsed = with_language(Lang::It, || parse_metadata(BufReader::new(&input[..]), 8192, 100).unwrap());
        let gimp = &parsed.apps["gimp"];
        assert_eq!(gimp.name, "Programma di ritocco");
        assert_eq!(gimp.summary, "Create \"images\"");
        assert_eq!(gimp.categories, vec!["Graphics", "2DGraphics"]);
        assert_eq!(gimp.id, "org.gimp.GIMP");
        let tom = &parsed.apps["tom"];
        assert_eq!(tom.name, "Tom's App");
        assert_eq!(tom.summary, "");
        assert!(!parsed.apps.contains_key("gimp-data"));
        assert_eq!(display_text("  spaced   out\ttext ", 80), "spaced out text");
        assert_eq!(display_text("a very long value", 6), "a very");
        for unsupported in [">-", "|", "[a, b]", "*alias", "'unterminated", "\"bad \\x escape\""] { assert_eq!(display_text(unsupported, 80), "", "{unsupported}"); }
    }

    #[test]
    fn flatpak_components_read_own_fields_entities_and_categories() {
        let xml = r#"<components>
          <component type="desktop-application"><id>org.example.Chat</id>
            <name>Chat &amp; Talk</name><name xml:lang="it">Chiacchiere &#233;</name><name xml:lang="de">Plaudern</name>
            <summary>Say hi</summary><summary xml:lang="it">Dì ciao</summary>
            <developer id="x"><name>Not the app name</name></developer>
            <categories><category>Network</category><category>Chat</category></categories>
            <provides><id>org.example.Nested</id></provides></component>
          <component type="addon"><id>org.example.Chat.Plugin</id><name>Plugin</name></component>
          <component type="desktop-application"><id>bad id</id></component>
        </components>"#;
        let parsed = parse_flatpak_components(BufReader::new(xml.as_bytes())).unwrap();
        assert_eq!(parsed.len(), 2, "invalid ids are discarded");
        let chat = &parsed[0];
        assert!(chat.application);
        assert_eq!(chat.name.default, "Chat & Talk");
        assert_eq!(chat.name.best(), "Chat & Talk");
        assert_eq!(chat.summary.best(), "Say hi");
        crate::i18n::with_language(crate::i18n::Lang::It, || {
            assert_eq!(chat.name.best(), "Chiacchiere é");
            assert_eq!(chat.summary.best(), "Dì ciao");
        });
        assert_eq!(chat.categories, vec!["Network", "Chat"]);
        assert_eq!(chat.id, "org.example.Chat");
        assert!(!parsed[1].application);
        assert!(!parsed.iter().any(|c| c.id == "org.example.Nested"));
    }

    #[test]
    fn flatpak_description_reads_paragraphs_lists_and_prefers_italian() {
        let xml = r#"<components>
          <component type="desktop-application"><id>org.example.Other</id><description><p>Not this one</p></description></component>
          <component type="desktop-application"><id>org.example.Chat</id>
            <description>
              <p>Talk &amp; share.</p><p xml:lang="it">Parla e condividi.</p><p xml:lang="de">Sprechen.</p>
              <ul><li>Fast</li><li xml:lang="it">Veloce</li><li>Free &amp; open</li></ul>
              <p>Second paragraph.</p>
            </description>
          </component></components>"#;
        let default = crate::i18n::with_language(crate::i18n::Lang::It, || parse_flatpak_description(BufReader::new(xml.as_bytes()), "org.example.Chat").unwrap().unwrap());
        assert!(default.starts_with("Parla e condividi."), "{default}");
        assert!(default.contains("• Veloce"), "{default}");
        let plain = parse_flatpak_description(BufReader::new(xml.as_bytes()), "org.example.Chat").unwrap().unwrap();
        assert!(plain.starts_with("Talk & share."), "{plain}");
        assert!(plain.contains("• Fast") && !plain.contains("Veloce"), "{plain}");
        assert_eq!(parse_flatpak_description(BufReader::new(xml.as_bytes()), "org.example.Missing").unwrap(), None);
        let english = r#"<components><component type="desktop-application"><id>a.b.C</id><description><p>One.</p><p>Two &amp; three.</p><ul><li>x</li><li>y</li></ul></description></component></components>"#;
        assert_eq!(parse_flatpak_description(BufReader::new(english.as_bytes()), "a.b.C").unwrap().unwrap(), "One.\n\nTwo & three.\n\n• x\n• y");
    }

    #[test]
    fn flatpak_description_language_on_the_description_element() {
        let xml = r#"<components><component type="desktop-application"><id>org.gimp.GIMP</id>
            <description xml:lang="be-Cyrl"><p>Беларуская.</p></description>
            <description><p>English text.</p><ul><li>One</li></ul></description>
            <description xml:lang="it"><p>Testo italiano.</p></description>
            <description xml:lang="de"><p>Deutsch.</p></description></component>
          <component type="desktop-application"><id>org.example.NoItalian</id>
            <description xml:lang="fr"><p>Français.</p></description><description><p>Only default.</p></description></component></components>"#;
        let italian = |id: &str| crate::i18n::with_language(crate::i18n::Lang::It, || parse_flatpak_description(BufReader::new(xml.as_bytes()), id).unwrap().unwrap());
        assert_eq!(italian("org.gimp.GIMP"), "Testo italiano.");
        assert_eq!(italian("org.example.NoItalian"), "Only default.");
        assert_eq!(parse_flatpak_description(BufReader::new(xml.as_bytes()), "org.gimp.GIMP").unwrap().unwrap(), "English text.\n\n• One", "an English interface never shows the Italian text");
    }

    #[test]
    fn desktop_entries_count_as_apps_only_when_visible() {
        assert_eq!(desktop_entry_icon("[Desktop Entry]\nType=Application\nExec=foo\nIcon=foo-icon\n"), Some("foo-icon".to_string()));
        assert_eq!(desktop_entry_icon("[Desktop Entry]\nType=Application\nExec=foo\n"), Some(String::new()));
        assert_eq!(desktop_entry_icon("[Desktop Entry]\nType=Application\nExec=foo\nNoDisplay=true\n"), None);
        assert_eq!(desktop_entry_icon("[Desktop Entry]\nType=Application\nHidden=true\nExec=foo\n"), None);
        assert_eq!(desktop_entry_icon("[Desktop Entry]\nType=Link\nExec=foo\n"), None);
        assert_eq!(desktop_entry_icon("[Desktop Entry]\nType=Application\n"), None, "without Exec it cannot be started");
        // Keys of another group (a desktop action) must not make the main entry visible or hidden.
        assert_eq!(desktop_entry_icon("[Desktop Entry]\nType=Application\nExec=foo\n[Desktop Action x]\nNoDisplay=true\nIcon=other\n"), Some(String::new()));
        assert!(apt_desktop_file("../../etc/passwd").is_none() && apt_desktop_file("-x").is_none());
        for invalid in ["../x", "a/b", "-x", "firefox.desktop", "a+b", ""] { assert!(snap_desktop_file(invalid).is_none(), "{invalid}"); }
    }

    #[test]
    #[ignore = "requires dpkg and installed desktop apps"]
    fn real_apt_packages_have_their_visible_launcher() {
        for (package, expected) in [("chatgpt", "chatgpt.desktop"), ("baobab", "org.gnome.baobab.desktop")] {
            let launcher = apt_desktop_file(package).unwrap_or_else(|| panic!("no launcher for {package}"));
            assert_eq!(launcher.file_name().unwrap().to_str().unwrap(), expected);
        }
        assert!(apt_desktop_file("libc6").is_none(), "a library has no launcher");
        assert!(snap_desktop_file("copilot-cli").is_none(), "a command-line snap has no launcher");
        assert!(snap_desktop_file("snap-store").is_some(), "a graphical snap has its launcher");
        assert!(apt_desktop_file("not-installed-at-all-xyz").is_none());
    }

    #[test]
    #[ignore = "requires Ubuntu AppStream, dpkg and a per-user Flathub catalog"]
    fn real_local_gimp_catalog_and_logos() {
        assert!(apt_is_application("gimp"));
        assert!(!apt_is_application("gimp-data"));
        assert!(apt_is_application("chatgpt"));
        let apt = local_icon("apt", "gimp", None, None).unwrap();
        let flatpak = local_icon("flatpak", "org.gimp.GIMP", Some("user"), Some("flathub")).unwrap();
        assert!(apt.starts_with("data:image/png;base64,"));
        assert!(flatpak.starts_with("data:image/png;base64,"));
        assert_eq!(flatpak_is_application("org.gimp.GIMP", Some("user"), Some("flathub")), Some(true));
        assert_eq!(flatpak_is_application("org.gimp.GIMP.Manual", Some("user"), Some("flathub")), Some(false));
        println!("GIMP classified; gimp-data excluded; installed third-party desktop classified; APT/Flatpak PNG logos {} / {} bytes", apt.len(), flatpak.len());
    }
}
