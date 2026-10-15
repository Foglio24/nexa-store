//! Store browsing: category pages and the randomised home page.
//!
//! The same application often exists in APT, Flatpak and the Snap Store. Browsing groups
//! those entries (same AppStream id, or same normalised name) so the frontend can show one
//! card per app and let the user choose the source. Nothing here runs a package manager
//! command except the read-only listings used to flag what is already installed.
use crate::appimage;
use crate::backend::{self, Package, Source};
use crate::catalog::{self, CatalogApp};
use crate::categories;
use crate::popular;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;

const SNAP_TTL: Duration = Duration::from_secs(900);
const SNAP_RETRY: Duration = Duration::from_secs(60);
const HERO_COUNT: usize = 4;
const PICK_COUNT: usize = 12;
const PICK_POPULAR: usize = 9;
const SECTION_COUNT: usize = 3;
const SECTION_SIZE: usize = 6;
const SECTION_POPULAR: usize = 4;
const MAX_PAGE: usize = 120;

/// Small deterministic generator (SplitMix64): the same seed always gives the same page,
/// so "load more" stays consistent, while a new seed gives a new selection.
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self { Self(seed ^ 0x9E37_79B9_7F4A_7C15) }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, bound: usize) -> usize { (self.next() % bound.max(1) as u64) as usize }
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() { let other = self.below(index + 1); items.swap(index, other); }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeSection { pub category: String, pub packages: Vec<Package> }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreHome { pub hero: Vec<Package>, pub picks: Vec<Package>, pub sections: Vec<HomeSection> }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryPage { pub packages: Vec<Package>, pub total: usize }

#[derive(Clone)]
struct Item {
    package: Package,
    has_icon: bool,
    /// Worth proposing on the home page. Server and IoT snaps are listed in their category only.
    featurable: bool,
    cats: Vec<&'static str>,
    name_key: String,
    id_key: String,
}

struct Group { name: String, items: Vec<Item>, popular: bool }
impl Group {
    fn has_icon(&self) -> bool { self.items.iter().any(|item| item.has_icon) }
    fn has_category(&self, category: &str) -> bool { self.items.iter().any(|item| item.cats.contains(&category)) }
    fn installed(&self) -> bool { self.items.iter().any(|item| item.package.installed) }
    fn summary(&self) -> &str { self.items.iter().map(|item| item.package.summary.as_str()).find(|s| !s.is_empty()).unwrap_or("") }
    fn primary_category(&self) -> &'static str { self.items.iter().find_map(|item| item.cats.first().copied()).unwrap_or("utilities") }
}

// ---- Snap Store catalogue ---------------------------------------------------------------

struct SnapCatalog { created: Instant, ttl: Duration, entries: Vec<Item> }
static SNAPS: OnceLock<Mutex<Option<SnapCatalog>>> = OnceLock::new();

fn load_snaps() -> Vec<Item> {
    let mut merged: HashMap<String, Item> = HashMap::new();
    for chunk in categories::SNAP_SECTIONS.chunks(5) {
        let rows: Vec<Vec<(&'static str, serde_json::Value)>> = std::thread::scope(|scope| {
            let handles: Vec<_> = chunk.iter().map(|section| scope.spawn(move || {
                backend::snap_get(&format!("/v2/find?section={}", backend::uri_component(section))).ok()
                    .and_then(|value| value.as_array().cloned()).unwrap_or_default()
                    .into_iter().map(|row| (*section, row)).collect::<Vec<_>>()
            })).collect();
            handles.into_iter().map(|handle| handle.join().unwrap_or_default()).collect()
        });
        for (section, row) in rows.into_iter().flatten() {
            if row["type"].as_str() != Some("app") || row["confinement"].as_str() != Some("strict") || row["devmode"].as_bool().unwrap_or(false) { continue; }
            let Ok(package) = backend::snap_package(&row, false, false) else { continue; };
            if package.kind != "application" || package.icon_url.is_none() { continue; }
            let category = categories::from_snap_section(section);
            let entry = merged.entry(package.id.clone()).or_insert_with(|| Item {
                name_key: categories::name_key(&package.name),
                id_key: package.app_id.clone().unwrap_or_default(),
                cats: Vec::new(), has_icon: true, featurable: true, package,
            });
            if matches!(section, "server-and-cloud" | "devices-and-iot") { entry.featurable = false; }
            if let Some(category) = category { if !entry.cats.contains(&category) { entry.cats.push(category); } }
        }
    }
    let mut entries: Vec<Item> = merged.into_values().collect();
    for item in &mut entries {
        for category in &item.package.categories {
            if let Some(known) = categories::CATEGORY_IDS.iter().find(|id| *id == category) { if !item.cats.contains(known) { item.cats.push(known); } }
        }
        item.package.categories = item.cats.iter().map(|c| c.to_string()).collect();
    }
    entries.sort_by(|a, b| a.package.id.cmp(&b.package.id));
    entries
}

fn snaps() -> Vec<Item> {
    if !backend::available("/usr/bin/snap") { return Vec::new(); }
    let mut cache = SNAPS.get_or_init(|| Mutex::new(None)).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.as_ref().map(|c| c.created.elapsed() >= c.ttl).unwrap_or(true) {
        let entries = load_snaps();
        // An empty result usually means snapd or the network is not ready: retry soon.
        let ttl = if entries.is_empty() { SNAP_RETRY } else { SNAP_TTL };
        *cache = Some(SnapCatalog { created: Instant::now(), ttl, entries });
    }
    cache.as_ref().map(|c| c.entries.clone()).unwrap_or_default()
}

/// Load every catalogue once, in the background, so the first visit to Explore is instant.
pub fn warm_up() {
    // The AppImage catalogue comes from the network: do not make the local ones wait for it.
    let remote = std::thread::spawn(appimage::warm_up);
    catalog::warm_up();
    let _ = snaps();
    let _ = remote.join();
}

// ---- Installed state --------------------------------------------------------------------

#[derive(Default)]
struct Installed { apt: HashSet<String>, flatpak: HashMap<String, Package>, snap: HashSet<String> }

fn installed_state() -> Installed {
    let mut state = Installed::default();
    if let Ok(packages) = backend::apt_installed() { state.apt = packages.iter().map(|p| backend::base_id(&p.id).to_string()).collect(); }
    if backend::available("/usr/bin/flatpak") {
        if let Ok(packages) = backend::flatpak_installed() {
            for package in packages { state.flatpak.entry(backend::app_id(&package.id).to_string()).or_insert(package); }
        }
    }
    if backend::available("/usr/bin/snap") {
        if let Ok(records) = backend::snap_local_records() {
            state.snap = records.iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect();
        }
    }
    state
}

fn mark_installed(package: &mut Package, state: &Installed) {
    match package.source {
        Source::Apt => package.installed = state.apt.contains(&package.id),
        Source::Snap => package.installed = state.snap.contains(&package.id),
        // AppImage entries already know whether they are installed (see `appimage::catalog_packages`).
        Source::Appimage => {},
        Source::Flatpak => if let Some(installed) = state.flatpak.get(&package.id) {
            // Keep the display data, but use the installed reference and scope so removal works.
            package.id = installed.id.clone();
            package.remote = installed.remote.clone();
            package.scope = installed.scope.clone();
            package.version = installed.version.clone();
            package.installed = true;
        },
    }
}

// ---- Candidates and grouping ------------------------------------------------------------

fn package_from(app: &CatalogApp) -> Package {
    Package {
        id: app.id.clone(), name: app.name.clone(), summary: app.summary.clone(), version: String::new(),
        source: if app.source == "apt" { Source::Apt } else { Source::Flatpak },
        installed: false, update_available: false, remote: app.remote.clone(), scope: app.scope.clone(),
        publisher: None, channel: None, confinement: None, kind: "application".into(), icon_url: None,
        app_id: (!app.app_id.is_empty()).then(|| app.app_id.clone()),
        categories: app.categories.iter().map(|c| c.to_string()).collect(),
    }
}

fn catalog_item(app: &CatalogApp, state: &Installed) -> Item {
    let mut package = package_from(app);
    mark_installed(&mut package, state);
    Item { has_icon: app.has_icon, featurable: true, cats: app.categories.clone(), name_key: categories::name_key(&app.name), id_key: app.app_id.clone(), package }
}

fn includes(source: &str, wanted: &str) -> bool { source == "all" || source == wanted }

/// The AppImage catalogue as store items. Most of its apps are little known, so on the home page only
/// the well-known ones are proposed; every app is still listed in its category and in search.
fn appimage_items() -> Vec<Item> {
    appimage::catalog_packages().into_iter().map(|package| {
        let name_key = categories::name_key(&package.name);
        let cats: Vec<&'static str> = package.categories.iter().filter_map(|c| categories::CATEGORY_IDS.iter().find(|id| *id == c).copied()).collect();
        Item { has_icon: package.icon_url.is_some(), featurable: popular::is_popular(&name_key), cats, name_key, id_key: String::new(), package }
    }).collect()
}

/// Apps of one category (or of any category when `category` is `None`), from the chosen sources.
fn collect(category: Option<&str>, source: &str, state: &Installed) -> Vec<Item> {
    let matches = |cats: &[&'static str]| match category { Some(c) => cats.contains(&c), None => !cats.is_empty() };
    let mut items = Vec::new();
    if includes(source, "apt") {
        items.extend(catalog::apt_apps().iter().filter(|app| matches(&app.categories)).map(|app| catalog_item(app, state)));
    }
    if includes(source, "flatpak") && backend::available("/usr/bin/flatpak") {
        items.extend(catalog::flatpak_apps().iter().filter(|app| matches(&app.categories)).map(|app| catalog_item(app, state)));
    }
    if includes(source, "snap") {
        for mut item in snaps().into_iter().filter(|item| matches(&item.cats)) {
            mark_installed(&mut item.package, state);
            items.push(item);
        }
    }
    if includes(source, "appimage") {
        items.extend(appimage_items().into_iter().filter(|item| matches(&item.cats)));
    }
    items
}

/// Merge entries that are the same application: equal AppStream id or equal normalised name.
fn group(items: Vec<Item>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for item in items {
        let found = by_id.get(&item.id_key).filter(|_| !item.id_key.is_empty())
            .or_else(|| by_name.get(&item.name_key).filter(|_| !item.name_key.is_empty())).copied();
        let index = found.unwrap_or_else(|| { groups.push(Group { name: item.package.name.clone(), items: Vec::new(), popular: false }); groups.len() - 1 });
        if !item.id_key.is_empty() { by_id.entry(item.id_key.clone()).or_insert(index); }
        if !item.name_key.is_empty() { by_name.entry(item.name_key.clone()).or_insert(index); }
        groups[index].items.push(item);
    }
    for group in &mut groups { group.popular = group.items.iter().any(|item| popular::is_popular(&item.name_key)); }
    groups
}

/// Fill in icons and the application's own name for what is about to be shown.
fn finish(groups: &[&Group]) -> Vec<Package> {
    let mut packages: Vec<Package> = groups.iter().flat_map(|group| group.items.iter().map(|item| item.package.clone())).collect();
    backend::enrich_packages(&mut packages);
    packages
}

// ---- Public commands --------------------------------------------------------------------

pub fn browse_category(category: String, source: String, sort: String, seed: u64, offset: usize, limit: usize) -> Result<CategoryPage> {
    if !categories::is_category(&category) { return Err(tr!("Invalid category.")); }
    if !matches!(source.as_str(), "all" | "apt" | "flatpak" | "snap" | "appimage") { return Err(tr!("Invalid source: use all, apt, flatpak, snap or appimage.")); }
    let state = installed_state();
    let mut groups = group(collect(Some(&category), &source, &state));
    if sort == "random" { Rng::new(seed).shuffle(&mut groups); }
    else { groups.sort_by_key(|g| categories::name_key(&g.name)); }
    // Apps that have a logo come first; the sort above is kept within each part.
    groups.sort_by_key(|g| !g.has_icon());
    let total = groups.len();
    let page: Vec<&Group> = groups.iter().skip(offset).take(limit.clamp(1, MAX_PAGE)).collect();
    Ok(CategoryPage { packages: finish(&page), total })
}

#[derive(Debug, PartialEq, Eq)]
struct HomePlan { hero: Vec<usize>, picks: Vec<usize>, sections: Vec<(&'static str, Vec<usize>)> }

/// A package-style name such as `kube-apiserver`: lowercase words joined by hyphens.
fn looks_technical(name: &str) -> bool {
    name.contains('-') && !name.contains(' ') && !name.chars().any(char::is_uppercase)
}

/// Worth highlighting: has a logo and a description, a short readable name, and is not installed yet.
fn showcase(group: &Group) -> bool {
    group.has_icon() && !group.summary().is_empty() && group.name.chars().count() <= 40 && !group.installed()
        && group.items.iter().any(|item| item.featurable) && !looks_technical(&group.name)
}

/// Take `count` unused candidates: first up to `popular_share` well-known apps, then random ones,
/// then more well-known apps if there are not enough random ones.
fn take_mixed(popular: &[usize], other: &[usize], count: usize, popular_share: usize, used: &mut HashSet<usize>, accept: impl Fn(usize) -> bool) -> Vec<usize> {
    let mut chosen: Vec<usize> = Vec::new();
    let mut add = |source: &[usize], limit: usize, chosen: &mut Vec<usize>| {
        for &index in source {
            if chosen.len() >= limit { break; }
            if !used.contains(&index) && accept(index) { chosen.push(index); used.insert(index); }
        }
    };
    add(popular, popular_share.min(count), &mut chosen);
    add(other, count, &mut chosen);
    add(popular, count, &mut chosen);
    chosen
}

fn plan_home(groups: &[Group], rng: &mut Rng) -> HomePlan {
    let (mut popular, mut other): (Vec<usize>, Vec<usize>) = (0..groups.len()).filter(|&i| showcase(&groups[i])).partition(|&i| groups[i].popular);
    rng.shuffle(&mut popular);
    rng.shuffle(&mut other);
    let mut used: HashSet<usize> = HashSet::new();

    // Highlighted apps: well-known ones, from different categories when possible.
    let mut hero = Vec::new();
    let mut hero_categories: HashSet<&str> = HashSet::new();
    for &index in popular.iter().chain(&other) {
        if hero.len() == HERO_COUNT { break; }
        if hero_categories.insert(groups[index].primary_category()) { hero.push(index); used.insert(index); }
    }

    let mut picks = take_mixed(&popular, &other, PICK_COUNT, PICK_POPULAR, &mut used, |_| true);
    rng.shuffle(&mut picks);

    let mut available: Vec<&'static str> = categories::CATEGORY_IDS.iter().copied()
        .filter(|category| popular.iter().chain(&other).filter(|&&i| !used.contains(&i) && groups[i].has_category(category)).count() >= SECTION_SIZE).collect();
    rng.shuffle(&mut available);
    let mut sections = Vec::new();
    for category in available.into_iter().take(SECTION_COUNT) {
        let mut chosen = take_mixed(&popular, &other, SECTION_SIZE, SECTION_POPULAR, &mut used, |i| groups[i].has_category(category));
        rng.shuffle(&mut chosen);
        sections.push((category, chosen));
    }
    HomePlan { hero, picks, sections }
}

pub fn explore_home(seed: u64) -> Result<ExploreHome> {
    let state = installed_state();
    let groups = group(collect(None, "all", &state));
    let plan = plan_home(&groups, &mut Rng::new(seed));
    let pick = |indexes: &[usize]| -> Vec<Package> { finish(&indexes.iter().map(|&i| &groups[i]).collect::<Vec<_>>()) };
    Ok(ExploreHome {
        hero: pick(&plan.hero),
        picks: pick(&plan.picks),
        sections: plan.sections.iter().map(|(category, indexes)| HomeSection { category: category.to_string(), packages: pick(indexes) }).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(source: Source, id: &str, name: &str, app_id: &str, cats: &[&'static str], icon: bool, installed: bool) -> Item {
        Item {
            package: Package { id: id.into(), name: name.into(), summary: format!("{name} summary"), version: String::new(), source, installed,
                update_available: false, remote: None, scope: None, publisher: None, channel: None, confinement: None,
                kind: "application".into(), icon_url: None, app_id: None, categories: Vec::new() },
            has_icon: icon, featurable: true, cats: cats.to_vec(), name_key: categories::name_key(name), id_key: app_id.into(),
        }
    }

    #[test]
    fn rng_is_deterministic_and_seed_dependent() {
        let (mut a, mut b, mut c) = (Rng::new(7), Rng::new(7), Rng::new(8));
        let mut one: Vec<u32> = (0..50).collect();
        let mut two = one.clone();
        let mut three = one.clone();
        a.shuffle(&mut one); b.shuffle(&mut two); c.shuffle(&mut three);
        assert_eq!(one, two);
        assert_ne!(one, three);
        let mut sorted = one.clone(); sorted.sort();
        assert_eq!(sorted, (0..50).collect::<Vec<u32>>(), "no element lost or duplicated");
    }

    #[test]
    fn same_app_in_several_sources_forms_one_group() {
        let items = vec![
            item(Source::Apt, "gimp", "GIMP", "org.gimp.gimp", &["graphics"], true, false),
            item(Source::Flatpak, "org.gimp.GIMP", "GIMP", "org.gimp.gimp", &["graphics"], true, false),
            item(Source::Snap, "gimp", "G.I.M.P", "", &["graphics"], true, false),
            item(Source::Flatpak, "org.other.Draw", "Draw", "org.other.draw", &["graphics"], true, false),
            item(Source::Apt, "vlc", "VLC", "vlc", &["audio-video"], true, false),
            item(Source::Snap, "vlc", "vlc", "", &["audio-video"], true, false),
        ];
        let groups = group(items);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].items.len(), 3, "same AppStream id or same normalized name");
        assert_eq!(groups[2].items.len(), 2, "same normalized name");
        assert_eq!(groups[1].items.len(), 1);
    }

    #[test]
    fn home_plan_respects_quotas_and_never_repeats_an_app() {
        let mut items = Vec::new();
        for (number, category) in categories::CATEGORY_IDS.iter().enumerate() {
            for index in 0..20 {
                items.push(item(Source::Flatpak, &format!("org.app{number}.App{index}"), &format!("App {number}-{index}"), &format!("org.app{number}.app{index}"), &[category], true, false));
            }
        }
        items.push(item(Source::Apt, "noicon", "No icon", "", &["games"], false, false));
        items.push(item(Source::Apt, "installed", "Installed", "", &["games"], true, true));
        let groups = group(items);
        let plan = plan_home(&groups, &mut Rng::new(42));
        assert_eq!(plan.hero.len(), HERO_COUNT);
        assert_eq!(plan.picks.len(), PICK_COUNT);
        assert_eq!(plan.sections.len(), SECTION_COUNT);
        let mut seen = HashSet::new();
        for index in plan.hero.iter().chain(&plan.picks).chain(plan.sections.iter().flat_map(|(_, ids)| ids)) {
            assert!(seen.insert(*index), "app repeated");
            let group = &groups[*index];
            assert!(group.has_icon() && !group.installed(), "{}", group.name);
        }
        let hero_categories: HashSet<_> = plan.hero.iter().map(|&i| groups[i].primary_category()).collect();
        assert_eq!(hero_categories.len(), HERO_COUNT, "highlighted apps come from different categories");
        for (category, ids) in &plan.sections {
            assert_eq!(ids.len(), SECTION_SIZE);
            assert!(ids.iter().all(|&i| groups[i].has_category(category)));
        }
        assert_eq!(plan, plan_home(&groups, &mut Rng::new(42)), "same seed, same home");
        assert_ne!(plan, plan_home(&groups, &mut Rng::new(43)), "new seed, new proposals");
    }

    #[test]
    fn home_prefers_known_apps_and_skips_server_components_and_technical_names() {
        let mut items = Vec::new();
        for index in 0..30 {
            items.push(item(Source::Flatpak, &format!("org.x.Obscure{index}"), &format!("Obscure Tool {index}"), &format!("org.x.obscure{index}"), &["utilities"], true, false));
        }
        for name in ["Firefox", "VLC", "GIMP", "Inkscape", "Krita", "Blender", "Audacity", "Steam", "Kdenlive", "Okular", "Thunderbird", "Telegram Desktop"] {
            items.push(item(Source::Apt, &name.to_lowercase(), name, "", &["utilities"], true, false));
        }
        let mut server = item(Source::Snap, "kube-apiserver", "Kube Server", "", &["utilities"], true, false);
        server.featurable = false;
        items.push(server);
        items.push(item(Source::Apt, "kube-scheduler", "kube-scheduler", "", &["utilities"], true, false));
        let groups = group(items);
        let plan = plan_home(&groups, &mut Rng::new(5));
        let shown: Vec<&Group> = plan.hero.iter().chain(&plan.picks).map(|&i| &groups[i]).collect();
        assert!(shown.iter().all(|g| g.name != "Kube Server" && g.name != "kube-scheduler"));
        assert!(plan.hero.iter().all(|&i| groups[i].popular), "highlighted apps are well-known ones");
        let popular_picks = plan.picks.iter().filter(|&&i| groups[i].popular).count();
        assert!(popular_picks >= PICK_POPULAR, "{popular_picks} well-known apps out of {}", plan.picks.len());
        assert!(plan.picks.iter().any(|&i| !groups[i].popular), "there is room for random discoveries");
        assert!(looks_technical("kube-apiserver") && !looks_technical("Kube-Apiserver") && !looks_technical("Cool App"));
    }

    #[test]
    #[ignore = "requires Ubuntu AppStream data; Flatpak and Snap sources are used when present"]
    fn real_home_and_categories() {
        let started = Instant::now();
        let home = explore_home(1).unwrap();
        println!("home: {:?} ({} highlighted, {} to discover, sections {:?})", started.elapsed(), home.hero.len(), home.picks.len(),
            home.sections.iter().map(|s| (&s.category, s.packages.len())).collect::<Vec<_>>());
        assert!(!home.hero.is_empty() && !home.picks.is_empty() && !home.sections.is_empty());
        for package in home.hero.iter().chain(&home.picks) {
            assert!(package.icon_url.is_some(), "{} has no logo", package.name);
            assert!(!package.installed, "{} is already installed", package.name);
        }
        let other = explore_home(2).unwrap();
        let names = |h: &ExploreHome| h.picks.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        assert_ne!(names(&home), names(&other), "a new seed must propose different apps");
        for category in categories::CATEGORY_IDS {
            let started = Instant::now();
            let page = browse_category(category.into(), "all".into(), "name".into(), 1, 0, 24).unwrap();
            let sources: HashSet<_> = page.packages.iter().map(|p| format!("{:?}", p.source)).collect();
            let with_icon = page.packages.iter().filter(|p| p.icon_url.is_some()).count();
            println!("{category:<12} groups={:<5} packages={:<3} with logo={:<3} sources={:?} {:?}", page.total, page.packages.len(), with_icon, sources, started.elapsed());
            assert!(page.total > 0, "{category} is empty");
        }
        let snap_only = browse_category("games".into(), "snap".into(), "random".into(), 3, 0, 12).unwrap();
        println!("games, Snap only: {} groups, example {:?}", snap_only.total, snap_only.packages.iter().take(3).map(|p| &p.name).collect::<Vec<_>>());
    }

    #[test]
    fn category_page_arguments_are_validated() {
        assert!(browse_category("../etc".into(), "all".into(), "name".into(), 1, 0, 10).is_err());
        assert!(browse_category("games".into(), "pip".into(), "name".into(), 1, 0, 10).is_err());
    }
}
