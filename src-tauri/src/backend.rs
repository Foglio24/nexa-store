//! Ubuntu package operations. All process arguments are explicit; no shell is used.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{atomic::{AtomicBool, Ordering}, mpsc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);
const MUTATION_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const PLAN_TTL: Duration = Duration::from_secs(120);
static PLANS: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
static MUTATING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source { Apt, Flatpak, Snap }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub version: String,
    pub source: Source,
    pub installed: bool,
    pub update_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confinement: Option<String>,
    #[serde(default = "default_package_kind")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// Normalised AppStream id, used to recognise the same app in another source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Store category ids (`giochi`, `grafica`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
}

fn default_package_kind() -> String { "package".into() }

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub ubuntu: bool,
    pub os_name: String,
    pub apt_available: bool,
    pub flatpak_available: bool,
    pub snap_available: bool,
    pub flatpak_remotes: Vec<String>,
    /// User Flatpak apps are installed (or can be) but this session does not search their launcher
    /// folder, so they will not show up in the app menu until the next login.
    pub flatpak_menu_hidden: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageDetails {
    pub package: Package,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub token: String,
    pub title: String,
    pub summary: String,
    pub command: String,
    pub details: Vec<String>,
    pub requires_admin: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult { pub success: bool, pub output: String }

#[derive(Debug, Clone)]
struct Invocation { program: String, args: Vec<String> }
impl Invocation {
    fn new(program: &str, args: &[&str]) -> Self {
        Self { program: program.into(), args: args.iter().map(|s| (*s).into()).collect() }
    }
    fn display(&self) -> String {
        // This representation is display-only. Execution always uses Command::args.
        std::iter::once(self.program.as_str()).chain(self.args.iter().map(String::as_str))
            .map(|s| if s.chars().all(|c| c.is_ascii_alphanumeric() || "-/._:=+~".contains(c)) {
                s.to_string()
            } else { format!("'{}'", s.replace('\'', "'\\''")) }).collect::<Vec<_>>().join(" ")
    }
}

struct Captured { success: bool, text: String }

/// Wait for a child process. If an elevated process cannot be killed at the time limit, the mutation
/// gate stays locked until that process really exits.
fn wait_with_timeout(mut child: Child, timeout: Duration, mutation: bool) -> Result<ExitStatus> {
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|e| format!("Errore di attesa del processo: {e}"))? {
            Some(status) => return Ok(status),
            None if started.elapsed() >= timeout => {
                if child.kill().is_ok() {
                    let _ = child.wait();
                } else {
                    // pkexec may have changed uid: don't permit a second mutation
                    // merely because the original elevated process timed out.
                    thread::spawn(move || {
                        let _ = child.wait();
                        if mutation { MUTATING.store(false, Ordering::Release); }
                    });
                    return Err("Tempo massimo superato. Il processo è ancora attivo; nuove operazioni resteranno bloccate finché terminerà.".into());
                }
                return Err("Tempo massimo del comando superato; il processo è stato interrotto. Verifica lo stato del gestore pacchetti prima di riprovare.".into());
            },
            None => thread::sleep(Duration::from_millis(40)),
        }
    }
}

/// Run a command that changes the system. Its output goes to a private file, never to pipes owned
/// by this process: if the window is closed or the app is stopped while a package manager is
/// working, the package manager must not receive SIGPIPE and stop half-way through an installation.
/// It also runs in its own process group, so signals sent to the app's group do not reach it.
fn run_detached(inv: &Invocation, timeout: Duration) -> Result<Captured> {
    use std::os::unix::{fs::OpenOptionsExt, process::CommandExt};
    let mut random = [0u8; 8];
    getrandom::getrandom(&mut random).map_err(|e| format!("Impossibile preparare il file di output: {e}"))?;
    let directory = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_dir()).unwrap_or_else(std::env::temp_dir);
    let path = directory.join(format!("nexa-store-{}-{}.log", std::process::id(), random.iter().map(|b| format!("{b:02x}")).collect::<String>()));
    let file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)
        .map_err(|e| format!("Impossibile creare il file di output: {e}"))?;
    let errors = file.try_clone().map_err(|e| format!("Impossibile preparare il file di output: {e}"))?;
    let spawned = Command::new(&inv.program).args(&inv.args)
        .env("LC_ALL", "C").env("LANG", "C").env("TERM", "dumb")
        .stdin(Stdio::null()).stdout(Stdio::from(file)).stderr(Stdio::from(errors))
        .process_group(0).spawn();
    let child = match spawned {
        Ok(child) => child,
        Err(e) => { let _ = fs::remove_file(&path); return Err(format!("Impossibile avviare {}: {e}", inv.program)); }
    };
    let status = wait_with_timeout(child, timeout, true);
    let mut data = Vec::new();
    if let Ok(output) = fs::File::open(&path) { let _ = output.take(OUTPUT_LIMIT as u64 + 1).read_to_end(&mut data); }
    let _ = fs::remove_file(&path);
    let truncated = data.len() > OUTPUT_LIMIT;
    data.truncate(OUTPUT_LIMIT);
    let mut text = String::from_utf8_lossy(&data).to_string();
    if truncated { text.push_str("\n[output troncato a 2 MiB]\n"); }
    Ok(Captured { success: status?.success(), text })
}

/// Drain both pipes concurrently, keeping memory bounded even for noisy tools. System changes
/// (`mutation`) use `run_detached` instead.
fn run(inv: &Invocation, timeout: Duration, mutation: bool) -> Result<Captured> {
    if mutation { return run_detached(inv, timeout); }
    let mut child = Command::new(&inv.program).args(&inv.args)
        .env("LC_ALL", "C").env("LANG", "C").env("TERM", "dumb")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("Impossibile avviare {}: {e}", inv.program))?;
    let (tx, rx) = mpsc::channel();
    for pipe in [child.stdout.take().map(|x| Box::new(x) as Box<dyn Read + Send>),
        child.stderr.take().map(|x| Box::new(x) as Box<dyn Read + Send>)].into_iter().flatten() {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut pipe = pipe;
            let mut data = Vec::new();
            let mut buf = [0u8; 8192];
            let mut truncated = false;
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let keep = n.min(OUTPUT_LIMIT.saturating_sub(data.len()));
                        data.extend_from_slice(&buf[..keep]);
                        truncated |= keep < n;
                    }
                }
            }
            let mut text = String::from_utf8_lossy(&data).to_string();
            if truncated { text.push_str("\n[output troncato a 2 MiB]\n"); }
            let _ = tx.send(text);
        });
    }
    drop(tx);
    let status = wait_with_timeout(child, timeout, mutation)?;
    let mut parts = Vec::new();
    for _ in 0..2 {
        if let Ok(part) = rx.recv_timeout(Duration::from_secs(2)) {
            if !part.trim().is_empty() { parts.push(part); }
        }
    }
    Ok(Captured { success: status.success(), text: parts.join("\n") })
}

fn query(program: &str, args: &[&str]) -> Result<String> {
    let result = run(&Invocation::new(program, args), QUERY_TIMEOUT, false)?;
    if result.text.contains("[output troncato a 2 MiB]") { return Err("La risposta del gestore pacchetti supera il limite di sicurezza. Restringi la ricerca.".into()); }
    if result.success { Ok(result.text) } else {
        Err(format!("{} non riuscito:\n{}", program, result.text.trim()))
    }
}

pub(crate) fn available(path: &str) -> bool { fs::metadata(path).map(|m| m.is_file()).unwrap_or(false) }
fn ubuntu_info() -> (bool, String) {
    let content = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let fields: HashMap<_, _> = content.lines().filter_map(|l| l.split_once('='))
        .map(|(k,v)| (k, v.trim_matches('"'))).collect();
    (fields.get("ID") == Some(&"ubuntu"), fields.get("PRETTY_NAME").unwrap_or(&"Linux sconosciuto").to_string())
}
fn ensure_ubuntu() -> Result<()> {
    if ubuntu_info().0 { Ok(()) } else { Err("Nexa Store supporta soltanto Ubuntu.".into()) }
}

fn selected(source: &str) -> Result<(bool, bool, bool)> {
    match source { "all" => Ok((true,true,true)), "apt" => Ok((true,false,false)), "flatpak" => Ok((false,true,false)), "snap" => Ok((false,false,true)),
        _ => Err("Sorgente non valida: usa all, apt, flatpak oppure snap.".into()) }
}

fn apt_id(id: &str) -> Result<()> {
    let (name, arch) = id.split_once(':').map(|(a,b)| (a,Some(b))).unwrap_or((id,None));
    if name.len() < 2 || id.len() > 255 || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-".contains(&b))
        || arch.map(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')).unwrap_or(false) {
        return Err("Identificatore APT non valido.".into());
    }
    Ok(())
}

fn flatpak_id(id: &str) -> Result<()> {
    if id.starts_with("app/") {
        let parts: Vec<_> = id.split('/').collect();
        if parts.len() != 4 { return Err("Riferimento Flatpak non valido.".into()); }
        flatpak_app_id(parts[1])?;
        for part in &parts[2..] {
            if part.is_empty() || part.len() > 128 || part.starts_with('-') ||
                !part.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) {
                return Err("Architettura o ramo Flatpak non valido.".into());
            }
        }
        Ok(())
    } else { flatpak_app_id(id) }
}

fn flatpak_app_id(id: &str) -> Result<()> {
    if id.len() < 3 || id.len() > 255 || !id.contains('.') || !id.as_bytes()[0].is_ascii_alphabetic()
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || id.split('.').any(str::is_empty) {
        Err("Identificatore Flatpak non valido.".into())
    } else { Ok(()) }
}

fn remote_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 128 || !id.as_bytes()[0].is_ascii_alphanumeric()
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) {
        Err("Nome del repository Flatpak non valido.".into())
    } else { Ok(()) }
}

fn scope_arg(scope: Option<&str>) -> Result<&'static str> {
    match scope.unwrap_or("user") { "user" => Ok("--user"), "system" => Ok("--system"),
        _ => Err("Ambito Flatpak non valido.".into()) }
}
pub(crate) fn base_id(id: &str) -> &str { id.split(':').next().unwrap_or(id) }
pub(crate) fn app_id(id: &str) -> &str { if id.starts_with("app/") { id.split('/').nth(1).unwrap_or(id) } else { id } }

#[derive(Debug, Clone, PartialEq, Eq)]
struct Remote { name: String, scope: String, url: String, verified: bool }
fn remotes() -> Result<Vec<Remote>> {
    let mut result = Vec::new();
    for (scope, arg) in [("user", "--user"), ("system", "--system")] {
        for line in query("/usr/bin/flatpak", &["remotes", arg, "--columns=name,url,options"] )?.lines() {
            let mut cols = line.split('\t');
            let name = cols.next().unwrap_or("").trim();
            if name.is_empty() { continue; }
            remote_id(name)?;
            let url = cols.next().unwrap_or("");
            let options = cols.next().unwrap_or("");
            result.push(Remote { name: name.into(), scope: scope.into(), url: url.into(), verified: !options.contains("no-gpg-verify") });
        }
    }
    Ok(result)
}

/// The graphical session takes its application folders from `XDG_DATA_DIRS` once, at login. If the
/// user Flatpak export folder did not exist then, Flatpak apps installed later do not appear in the
/// app grid, the dock or the search until the next login.
pub fn flatpak_menu_hidden() -> bool {
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let Some(exports) = data_home.map(|path| path.join("flatpak/exports/share")) else { return false; };
    if !exports.is_dir() { return false; }
    !std::env::var("XDG_DATA_DIRS").unwrap_or_default().split(':').any(|dir| std::path::Path::new(dir.trim_end_matches('/')) == exports)
}

pub fn system_info() -> Result<SystemInfo> {
    let (ubuntu, os_name) = ubuntu_info();
    let apt_available = available("/usr/bin/apt-get") && available("/usr/bin/apt-cache") && available("/usr/bin/dpkg-query");
    let flatpak_available = available("/usr/bin/flatpak");
    let snap_available = available("/usr/bin/snap") && snap_get("/v2/system-info").is_ok();
    let mut warnings = Vec::new();
    if !ubuntu { warnings.push("Sistema non supportato: è richiesto Ubuntu.".into()); }
    if !available("/usr/bin/pkexec") { warnings.push("pkexec non disponibile: le operazioni APT richiedono policykit-1/pkexec.".into()); }
    if !flatpak_available { warnings.push("Flatpak non è installato. La ricerca APT resta disponibile.".into()); }
    if !snap_available { warnings.push("Snap non è disponibile: verifica snapd e il servizio /run/snapd.socket.".into()); }
    let mut flatpak_remotes = Vec::new();
    if flatpak_available {
        match remotes() {
            Ok(rows) => {
                if !rows.iter().any(|r| r.scope == "user") {
                    warnings.push("Nessun repository Flatpak utente configurato. Per installare app Flatpak aggiungi prima un repository attendibile con flatpak --user remote-add.".into());
                }
                for r in rows {
                    if !r.verified { warnings.push(format!("Repository Flatpak {}: verifica GPG disattivata; le operazioni saranno bloccate.", r.name)); }
                    flatpak_remotes.push(format!("{}:{}", r.scope, r.name));
                }
            },
            Err(e) => warnings.push(e),
        }
    }
    Ok(SystemInfo { ubuntu, os_name, apt_available, flatpak_available, snap_available, flatpak_remotes, flatpak_menu_hidden: flatpak_available && flatpak_menu_hidden(), warnings })
}

fn parse_dpkg(text: &str) -> Vec<Package> {
    text.lines().filter_map(|line| {
        let c: Vec<_> = line.split('\t').collect();
        if c.len() < 4 || !c[3].starts_with("ii") { return None; }
        Some(Package { id: c[0].into(), name: c[0].into(), version: c[1].into(), summary: c[2].into(),
            source: Source::Apt, installed: true, update_available: false, remote: None, scope: None, publisher: None, channel: None, confinement: None, kind:default_package_kind(),icon_url:None,app_id:None,categories:Vec::new() })
    }).collect()
}
pub(crate) fn apt_installed() -> Result<Vec<Package>> {
    Ok(parse_dpkg(&query("/usr/bin/dpkg-query", &["-W", "-f=${binary:Package}\t${Version}\t${binary:Summary}\t${db:Status-Abbrev}\n"])?))
}
fn parse_apt_updates(text: &str) -> Vec<Package> {
    text.lines().filter_map(|line| {
        let (id, rest) = line.split_once('/')?;
        if !rest.contains("[upgradable from:") || apt_id(id).is_err() { return None; }
        let cols: Vec<_> = rest.split_whitespace().collect();
        Some(Package { id: id.into(), name: id.into(), summary: "Aggiornamento APT disponibile".into(),
            version: cols.get(1).unwrap_or(&"").to_string(), source: Source::Apt,
            installed: true, update_available: true, remote: None, scope: None, publisher: None, channel: None, confinement: None, kind:default_package_kind(),icon_url:None,app_id:None,categories:Vec::new() })
    }).collect()
}
fn apt_updates() -> Result<Vec<Package>> {
    Ok(parse_apt_updates(&query("/usr/bin/apt", &["list", "--upgradable"])?))
}

fn parse_control(text: &str) -> Vec<BTreeMap<String, String>> {
    let mut result = Vec::new();
    let mut current = BTreeMap::<String,String>::new();
    let mut key = String::new();
    for line in text.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if !current.is_empty() { result.push(std::mem::take(&mut current)); }
            key.clear();
        } else if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(value) = current.get_mut(&key) {
                value.push('\n');
                if line.trim() != "." { value.push_str(line.trim_start()); }
            }
        } else if let Some((k,v)) = line.split_once(':') {
            key = k.into(); current.insert(key.clone(),v.trim_start().into());
        }
    }
    result
}

fn validate_query(query: &str) -> Result<&str> {
    let query = query.trim();
    if query.chars().count() < 2 || query.len() > 120 || query.chars().any(char::is_control) {
        Err("La ricerca deve contenere da 2 a 120 caratteri, senza caratteri di controllo.".into())
    } else { Ok(query) }
}
fn regex_literal(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if ".^$*+?()[]{}|\\".contains(ch) { result.push('\\'); }
        result.push(ch);
    }
    result
}

fn apt_search(query_text: &str) -> Result<Vec<Package>> {
    let pattern = regex_literal(query_text);
    let output = query("/usr/bin/apt-cache", &["search", "--names-only", "--", &pattern])?;
    let installed: HashMap<_,_> = apt_installed()?.into_iter().map(|p| (p.id.clone(),p)).collect();
    let updates: HashSet<_> = apt_updates()?.into_iter().map(|p| p.id).collect();
    let mut result: Vec<_> = output.lines().filter_map(|line| {
        let (id, summary) = line.split_once(" - ")?;
        if apt_id(id).is_err() { return None; }
        let current = installed.get(id).or_else(|| installed.values().find(|p| base_id(&p.id) == id));
        Some(Package { id: id.into(), name: id.into(), summary: summary.into(), version: current.map(|p| p.version.clone()).unwrap_or_default(),
            source: Source::Apt, installed: current.is_some(), update_available: updates.contains(id), remote: None, scope: None, publisher: None, channel: None, confinement: None, kind:default_package_kind(),icon_url:None,app_id:None,categories:Vec::new() })
    }).collect();
    result.sort_by_key(|p| (p.id != query_text, !crate::catalog::apt_is_application(base_id(&p.id)), !p.id.starts_with(query_text), !p.installed, p.id.clone()));
    result.truncate(80);
    if !result.is_empty() {
        let ids: Vec<_> = result.iter().map(|p| p.id.as_str()).collect();
        let mut args = vec!["show", "--no-all-versions", "--"];
        args.extend(ids);
        if let Ok(metadata) = query("/usr/bin/apt-cache", &args) {
            let versions: HashMap<_,_> = parse_control(&metadata).into_iter().filter_map(|mut m| Some((m.remove("Package")?,m.remove("Version")?))).collect();
            for p in &mut result { if let Some(v) = versions.get(&p.id) { p.version = v.clone(); } }
        }
    }
    Ok(result)
}

/// `flatpak list --app` prints the ref as `com.example.App/x86_64/stable`, while `remote-ls` and
/// `remote-info` print `app/com.example.App/x86_64/stable`. Everything in this module uses the long form.
fn normalize_ref(reference: &str) -> String {
    if reference.starts_with("app/") || reference.starts_with("runtime/") { reference.to_string() } else { format!("app/{reference}") }
}

fn parse_flatpak_list(text: &str, scope: &str) -> Vec<Package> {
    text.lines().filter_map(|line| {
        let c: Vec<_> = line.split('\t').collect();
        if c.len() < 6 { return None; }
        let reference = normalize_ref(c[5]);
        if flatpak_id(&reference).is_err() { return None; }
        Some(Package { id: reference, name: if c[1].is_empty() { c[0].into() } else { c[1].into() },
            summary: c[2].into(), version: c[3].into(), source: Source::Flatpak,
            installed: true, update_available: false, remote: Some(c[4].into()), scope: Some(scope.into()), publisher: None, channel: None, confinement: None, kind:default_package_kind(),icon_url:None,app_id:None,categories:Vec::new() })
    }).collect()
}
pub(crate) fn flatpak_installed() -> Result<Vec<Package>> {
    let mut result = Vec::new();
    for (scope, arg) in [("user", "--user"), ("system", "--system")] {
        result.extend(parse_flatpak_list(&query("/usr/bin/flatpak", &["list", arg, "--app", "--columns=application,name,description,version,origin,ref"] )?,scope));
    }
    Ok(result)
}
fn flatpak_updates() -> Result<Vec<Package>> {
    let installed = flatpak_installed()?;
    let mut result = Vec::new();
    for (scope, arg) in [("user", "--user"), ("system", "--system")] {
        if !installed.iter().any(|p| p.scope.as_deref() == Some(scope)) { continue; }
        let text = query("/usr/bin/flatpak", &["remote-ls", arg, "--updates", "--app", "--columns=application,version,ref"])?;
        for line in text.lines() {
            let c: Vec<_> = line.split('\t').collect();
            if c.len() < 3 { continue; }
            let reference = normalize_ref(c[2]);
            if let Some(package) = installed.iter().find(|p| p.id == reference && p.scope.as_deref() == Some(scope)) {
                let mut p = package.clone(); p.update_available = true; p.version = c[1].into(); result.push(p);
            }
        }
    }
    Ok(result)
}
fn flatpak_search(query_text: &str) -> Result<Vec<Package>> {
    let mut result = Vec::new();
    let configured = remotes()?;
    let installed = flatpak_installed()?;
    let updates = flatpak_updates()?;
    for (scope, arg) in [("user", "--user"), ("system", "--system")] {
        if !configured.iter().any(|r| r.scope == scope) { continue; }
        // AppStream search includes runtime/plugin extensions too. Only expose
        // actual application refs, which this phase can install and manage.
        let app_ids: HashSet<String> = query("/usr/bin/flatpak", &["remote-ls", arg, "--app", "--cached", "--columns=application"])?.lines().map(str::to_string).collect();
        let output = query("/usr/bin/flatpak", &["search", arg, "--columns=application,name,description,version,remotes", "--", query_text])?;
        for line in output.lines() {
            let c: Vec<_> = line.split('\t').collect();
            if c.len() < 5 || flatpak_app_id(c[0]).is_err() || !app_ids.contains(c[0]) { continue; }
            if let Some(p) = installed.iter().find(|p| app_id(&p.id) == c[0] && p.scope.as_deref() == Some(scope)) {
                let mut p = p.clone();
                p.update_available = updates.iter().any(|u| u.id == p.id && u.scope == p.scope);
                result.push(p);
            } else {
                for remote in c[4].split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if remote_id(remote).is_err() { continue; }
                    result.push(Package { id: c[0].into(), name: c[1].into(), summary: c[2].into(), version: c[3].into(), source: Source::Flatpak,
                        installed: false, update_available: false, remote: Some(remote.into()), scope: Some(scope.into()), publisher: None, channel: None, confinement: None, kind:default_package_kind(),icon_url:None,app_id:None,categories:Vec::new() });
                }
            }
        }
    }
    result.sort_by_key(|p| (!p.installed, p.name.to_lowercase(), p.scope.clone(), p.remote.clone()));
    result.dedup_by(|a,b| a.id == b.id && a.scope == b.scope && a.remote == b.remote);
    result.truncate(80);
    Ok(result)
}

// Snap uses snapd's documented, read-only REST API on a fixed local Unix
// socket. This avoids parsing localized, aligned CLI tables and never sends a
// POST/PUT/DELETE or credentials to the daemon.
pub(crate) fn uri_component(value: &str) -> String {
    let mut encoded=String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) { encoded.push(byte as char); }
        else { encoded.push_str(&format!("%{byte:02X}")); }
    }
    encoded
}

fn decode_http_response(data: &[u8]) -> Result<(u16, serde_json::Value)> {
    let offset=data.windows(4).position(|s|s == b"\r\n\r\n").ok_or("Risposta HTTP snapd incompleta.")?;
    if offset > 16384 { return Err("Intestazioni snapd oltre il limite consentito.".into()); }
    let header=std::str::from_utf8(&data[..offset]).map_err(|_|"Intestazioni snapd non valide.")?;
    let mut lines=header.lines();
    let status=lines.next().and_then(|l|l.split_whitespace().nth(1)).and_then(|s|s.parse::<u16>().ok()).ok_or("Stato HTTP snapd non valido.")?;
    let fields: HashMap<String,String> = lines.filter_map(|l|l.split_once(':')).map(|(k,v)|(k.to_ascii_lowercase(),v.trim().to_ascii_lowercase())).collect();
    let body=&data[offset+4..];
    let mut decoded=Vec::new();
    if fields.get("transfer-encoding").map(String::as_str) == Some("chunked") {
        let mut pos=0;
        loop {
            let end=body[pos..].windows(2).position(|s|s == b"\r\n").map(|n|pos+n).ok_or("Chunk HTTP snapd incompleto.")?;
            let size=std::str::from_utf8(&body[pos..end]).ok().and_then(|s|usize::from_str_radix(s.split(';').next().unwrap_or(""),16).ok()).ok_or("Dimensione chunk snapd non valida.")?;
            pos=end+2;
            if size == 0 { break; }
            if size > OUTPUT_LIMIT.saturating_sub(decoded.len()) || size > body.len().saturating_sub(pos) { return Err("Risposta snapd incompleta o troppo grande.".into()); }
            decoded.extend_from_slice(&body[pos..pos+size]); pos+=size;
            if body.get(pos..pos+2) != Some(b"\r\n") { return Err("Terminazione chunk snapd non valida.".into()); }
            pos+=2;
        }
    } else {
        if fields.contains_key("transfer-encoding") { return Err("Codifica HTTP snapd non supportata.".into()); }
        if let Some(length)=fields.get("content-length") {
            let length=length.parse::<usize>().map_err(|_|"Lunghezza HTTP snapd non valida.")?;
            if length != body.len() { return Err("Risposta snapd incompleta.".into()); }
        }
        if body.len() > OUTPUT_LIMIT { return Err("Risposta snapd oltre il limite di sicurezza.".into()); }
        decoded.extend_from_slice(body);
    }
    let json=serde_json::from_slice(&decoded).map_err(|e|format!("JSON snapd non valido: {e}"))?;
    Ok((status,json))
}

pub(crate) fn snap_get(path: &str) -> Result<serde_json::Value> {
    if !path.starts_with("/v2/") || path.bytes().any(|b|b.is_ascii_control() || b == b' ') {
        return Err("Percorso API snapd non valido.".into());
    }
    let mut socket=UnixStream::connect("/run/snapd.socket").map_err(|e|format!("snapd non raggiungibile: {e}"))?;
    socket.set_read_timeout(Some(Duration::from_secs(2))).map_err(|e|e.to_string())?;
    socket.set_write_timeout(Some(Duration::from_secs(5))).map_err(|e|e.to_string())?;
    socket.write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nConnection: close\r\n\r\n").as_bytes()).map_err(|e|format!("Richiesta snapd non riuscita: {e}"))?;
    let mut data=Vec::new();
    let mut buf=[0u8;8192];
    let started=Instant::now();
    loop {
        if started.elapsed() >= QUERY_TIMEOUT { return Err("Tempo massimo della richiesta snapd superato.".into()); }
        match socket.read(&mut buf) {
            Ok(0)=>break,
            Ok(n)=>{
                if data.len()+n > OUTPUT_LIMIT+16384 { return Err("Risposta snapd oltre il limite di sicurezza.".into()); }
                data.extend_from_slice(&buf[..n]);
            },
            Err(e) if matches!(e.kind(),std::io::ErrorKind::TimedOut|std::io::ErrorKind::WouldBlock)=>continue,
            Err(e)=>return Err(format!("Lettura snapd non riuscita: {e}")),
        }
    }
    let (status,value)=decode_http_response(&data)?;
    if status != 200 || value.get("type").and_then(|v|v.as_str()) != Some("sync") {
        return Err(format!("snapd: {}",value.pointer("/result/message").and_then(|v|v.as_str()).unwrap_or("richiesta non riuscita")));
    }
    value.get("result").cloned().ok_or("snapd non ha restituito un risultato.".into())
}

fn snap_name(id: &str) -> Result<()> {
    if id.len() < 2 || id.len() > 40 || id.starts_with('-') || id.ends_with('-') || id.contains("--")
        || !id.bytes().all(|b|b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || !id.bytes().any(|b|b.is_ascii_lowercase()) {
        Err("Nome Snap non valido: sono ammessi nomi canonici dello Store, senza opzioni, percorsi o istanze parallele.".into())
    } else { Ok(()) }
}
fn snap_string(value: &serde_json::Value, field: &str) -> String {
    value.get(field).and_then(|v|v.as_str()).unwrap_or("").to_string()
}
fn stable_snap_channel(channel: &str) -> Result<String> {
    if channel == "stable" { return Ok("latest/stable".into()); }
    let parts: Vec<_>=channel.split('/').collect();
    if parts.len() == 2 && parts[1] == "stable" && !parts[0].is_empty() && parts[0].len() <= 64
        && parts[0].as_bytes()[0].is_ascii_alphanumeric()
        && parts[0].bytes().all(|b|b.is_ascii_alphanumeric() || b".+-".contains(&b)) {
        Ok(channel.into())
    } else { Err("Sono supportati soltanto canali stable senza ramo temporaneo; il canale attuale non verrà cambiato automaticamente.".into()) }
}
fn snap_publisher(value: &serde_json::Value) -> String {
    let publisher=&value["publisher"];
    let user=snap_string(publisher,"username");
    let display=snap_string(publisher,"display-name");
    let state=if snap_string(publisher,"validation") == "verified" {"verificato"} else {"non verificato"};
    if display.is_empty() { format!("{user} · {state}") } else { format!("{display} (@{user}) · {state}") }
}
fn snap_app_id(value: &serde_json::Value) -> Option<String> {
    value.get("common-ids")?.as_array()?.iter().filter_map(|v|v.as_str()).map(crate::categories::id_key).find(|id|!id.is_empty())
}
fn snap_categories(value: &serde_json::Value) -> Vec<String> {
    let mut result: Vec<String>=Vec::new();
    for category in value.get("categories").and_then(|v|v.as_array()).into_iter().flatten() {
        if let Some(id)=category.get("name").and_then(|v|v.as_str()).and_then(crate::categories::from_snap_section) {
            if !result.iter().any(|existing|existing == id) {result.push(id.to_string());}
        }
    }
    result
}
fn snap_icon(value: &serde_json::Value) -> Option<String> {
    let url=value.get("icon")?.as_str()?;
    let authority=url.strip_prefix("https://")?.split('/').next()?;
    if url.len() > 8192 || url.chars().any(char::is_control) || authority.is_empty()
        || !authority.bytes().all(|b|b.is_ascii_alphanumeric() || b".-:".contains(&b)) {None} else {Some(url.into())}
}
pub(crate) fn snap_package(value: &serde_json::Value, installed: bool, update: bool) -> Result<Package> {
    let id=snap_string(value,"name"); snap_name(&id)?;
    let title=snap_string(value,"title");
    let channel=if installed {snap_string(value,"tracking-channel")} else {snap_string(value,"channel")};
    Ok(Package {id:id.clone(),name:if title.is_empty(){id}else{title},summary:snap_string(value,"summary"),version:snap_string(value,"version"),
        source:Source::Snap,installed,update_available:update,remote:Some("Snap Store".into()),scope:Some("system".into()),
        publisher:Some(snap_publisher(value)),channel:Some(if channel == "stable" {"latest/stable".into()} else {channel}),confinement:Some(snap_string(value,"confinement")),
        kind:if critical_snap(&snap_string(value,"name"),&snap_string(value,"type")) {"package".into()} else {"application".into()},icon_url:snap_icon(value),app_id:snap_app_id(value),categories:snap_categories(value)})
}
pub(crate) fn snap_local_records() -> Result<Vec<serde_json::Value>> {
    snap_get("/v2/snaps")?.as_array().cloned().ok_or("Elenco Snap installati non valido.".into())
}
fn snap_store_record(name: &str) -> Result<serde_json::Value> {
    snap_name(name)?;
    snap_get(&format!("/v2/find?name={}",uri_component(name)))?.as_array().and_then(|rows|rows.iter().find(|v|v["name"].as_str() == Some(name))).cloned().ok_or("Snap non disponibile nello Store.".into())
}
fn snap_update_records() -> Result<Vec<serde_json::Value>> {
    snap_get("/v2/find?select=refresh")?.as_array().cloned().ok_or("Elenco aggiornamenti Snap non valido.".into())
}
fn snap_installed() -> Result<Vec<Package>> {
    snap_local_records()?.iter().map(|v|snap_package(v,true,false)).collect()
}
fn snap_updates() -> Result<Vec<Package>> {
    let installed=snap_local_records()?;
    snap_update_records()?.iter().map(|v|{
        let mut package=snap_package(v,true,true)?;
        if let Some(local)=installed.iter().find(|l|l["name"] == v["name"]) { package.channel=Some(snap_string(local,"tracking-channel")); }
        Ok(package)
    }).collect()
}
fn snap_search(query_text: &str) -> Result<Vec<Package>> {
    let installed=snap_local_records()?;
    let updates:HashSet<String>=snap_update_records()?.iter().map(|v|snap_string(v,"name")).collect();
    let rows=snap_get(&format!("/v2/find?q={}",uri_component(query_text)))?;
    let rows=rows.as_array().ok_or("Risposta di ricerca Snap non valida.")?;
    let mut result=Vec::new();
    for value in rows.iter().take(80) {
        let name=snap_string(value,"name");
        let mut package=snap_package(value,false,false)?;
        if let Some(local)=installed.iter().find(|v|v["name"].as_str() == Some(&name)) {
            package=snap_package(local,true,updates.contains(&name))?;
            if package.icon_url.is_none() {package.icon_url=snap_icon(value);}
        }
        result.push(package);
    }
    result.sort_by_key(|p|(p.id != query_text,!p.installed,p.name.to_lowercase()));
    Ok(result)
}
fn snap_details(package: Package) -> Result<PackageDetails> {
    snap_name(&package.id)?;
    let installed=snap_local_records()?;
    let local=installed.iter().find(|v|v["name"].as_str() == Some(&package.id));
    let metadata=match snap_store_record(&package.id) {Ok(v)=>v,Err(e)=>match local {Some(v)=>v.clone(),None=>return Err(e)}};
    let mut canonical=snap_package(local.unwrap_or(&metadata),local.is_some(),false)?;
    if canonical.icon_url.is_none() {canonical.icon_url=snap_icon(&metadata);}
    if canonical.installed { canonical.update_available=snap_update_records()?.iter().any(|v|v["name"] == metadata["name"]); }
    Ok(PackageDetails {package:canonical,description:snap_string(&metadata,"description"),homepage:Some(format!("https://snapcraft.io/{}",package.id)),
        size:local.and_then(|v|v.get("installed-size")).or_else(||metadata.get("download-size")).and_then(|v|v.as_u64()).map(|n|format!("{:.1} MiB{}",n as f64 /1048576.0,if local.is_some(){" installati"}else{" da scaricare"})),
        license:metadata.get("license").and_then(|v|v.as_str()).map(str::to_string)})
}

fn critical_snap(name: &str, kind: &str) -> bool {
    kind != "app" || matches!(name,"core"|"bare"|"snapd"|"snap-store"|"firmware-updater"|"desktop-security-center"|"prompting-client"|"snapd-desktop-integration"|"gtk-common-themes")
        || name.strip_prefix("core").map(|s|!s.is_empty() && s.bytes().all(|b|b.is_ascii_digit())).unwrap_or(false)
        || name.strip_prefix("gnome-").and_then(|s|s.bytes().next()).map(|b|b.is_ascii_digit()).unwrap_or(false)
        || name.starts_with("mesa-") || name.starts_with("ubuntu-")
}
#[derive(Debug,Clone,PartialEq,Eq)]
struct SnapIdentity {name:String,store_id:String,revision:String,channel:String,confinement:String,publisher:String,kind:String,devmode:bool,status:String}
fn snap_identity(value: &serde_json::Value, channel: &str, store: bool) -> Result<SnapIdentity> {
    let name=snap_string(value,"name"); snap_name(&name)?;
    let mut identity=SnapIdentity {name,store_id:snap_string(value,"id"),revision:snap_string(value,"revision"),channel:stable_snap_channel(channel)?,
        confinement:snap_string(value,"confinement"),publisher:format!("{} · Store publisher ID: {}",snap_publisher(value),snap_string(&value["publisher"],"id")),
        kind:snap_string(value,"type"),devmode:value["devmode"].as_bool().unwrap_or(false),status:snap_string(value,"status")};
    if store {
        let release=value.get("channels").and_then(|c|c.get(&identity.channel)).ok_or("La versione stable selezionata non è disponibile nello Store.")?;
        identity.revision=snap_string(release,"revision");
        identity.confinement=snap_string(release,"confinement");
    }
    if critical_snap(&identity.name,&identity.kind) { return Err("Operazione Snap bloccata: base, kernel, gadget, snapd o componente di sistema richiede gli strumenti dedicati di Ubuntu.".into()); }
    if identity.confinement != "strict" || identity.devmode { return Err("Operazione Snap bloccata: sono supportate soltanto app con confinamento strict; classic e devmode richiedono un flusso dedicato.".into()); }
    if identity.store_id.len() != 32 || !identity.store_id.bytes().all(|b|b.is_ascii_alphanumeric())
        || identity.revision.is_empty() || identity.revision.len() > 20 || !identity.revision.bytes().all(|b|b.is_ascii_digit()) || identity.revision == "0" {
        return Err("Identità Store o revisione Snap non verificabile; nessuna operazione verrà eseguita.".into());
    }
    Ok(identity)
}
#[derive(Debug,PartialEq,Eq)]
struct SnapApproval {name:String,channel:String,installed:Option<SnapIdentity>,candidate:Option<SnapIdentity>}
fn snap_approval(name: &str, action: &str, channel: &str) -> Result<SnapApproval> {
    snap_name(name)?;
    let local=snap_local_records()?.into_iter().find(|v|v["name"].as_str() == Some(name));
    let installed=local.as_ref().map(|v|snap_identity(v,&snap_string(v,"tracking-channel"),false)).transpose()?;
    if action == "install" && local.is_some() { return Err("Questo Snap è già installato.".into()); }
    if action != "install" && local.is_none() { return Err("Questo Snap non è installato.".into()); }
    let candidate=if action == "remove" {None} else {Some(snap_identity(&snap_store_record(name)?,channel,true)?)};
    if let (Some(old),Some(new))=(&installed,&candidate) {
        if old.store_id != new.store_id || old.publisher != new.publisher { return Err("L'identità dello Snap installato differisce dallo Store; operazione bloccata.".into()); }
        if old.channel != channel { return Err("Il canale Snap è cambiato; prepara un nuovo piano.".into()); }
        if old.revision == new.revision { return Err("Nessun aggiornamento disponibile per questo Snap.".into()); }
    }
    Ok(SnapApproval {name:name.into(),channel:channel.into(),installed,candidate})
}
fn snap_plan(package: Option<Package>, action: &str) -> Result<(Invocation,Proof,Vec<String>,bool)> {
    if !available("/usr/bin/snap") { return Err("Snap non è installato.".into()); }
    snap_get("/v2/system-info")?;
    if action == "refresh" {
        return Ok((Invocation::new("/usr/bin/snap",&["refresh","--list"]),Proof::None,
            vec!["Controlla soltanto gli aggiornamenti disponibili nello Snap Store. Il comando snap refresh --list non aggiorna e non installa alcun pacchetto.".into()],false));
    }
    if !available("/usr/bin/pkexec") { return Err("pkexec non disponibile: necessario per modificare gli Snap.".into()); }
    let package=package.ok_or("Seleziona un'app Snap.")?;
    if package.source != Source::Snap { return Err("Sorgente del pacchetto non coerente.".into()); }
    snap_name(&package.id)?;
    // Read the actual tracking channel from snapd. Never trust IPC package flags
    // or silently move an existing app onto another track.
    let channel=if action == "install" {"latest/stable".into()} else {
        let local=snap_local_records()?.into_iter().find(|v|v["name"].as_str() == Some(&package.id)).ok_or("Snap non installato.")?;
        stable_snap_channel(&snap_string(&local,"tracking-channel"))?
    };
    let approved=snap_approval(&package.id,action,&channel)?;
    let identity=approved.candidate.as_ref().or(approved.installed.as_ref()).ok_or("Identità Snap non disponibile.")?;
    let verb=match action {"install"=>"install","remove"=>"remove","update"=>"refresh",_=>return Err("Azione Snap non valida.".into())};
    let mut args=vec!["/usr/bin/snap".into(),verb.into()];
    if action != "remove" {args.push(format!("--channel={channel}"));args.push(format!("--revision={}",identity.revision));}
    args.push("--".into()); args.push(package.id.clone());
    let details=vec![format!("App: {}. Snap Store ID: {}. Revisione approvata: {}. Canale: {}. Confinamento: {}.",package.id,identity.store_id,identity.revision,channel,identity.confinement),
        format!("Publisher dichiarato dallo Snap Store: {}. Nessuna opzione --classic, --devmode, --dangerous o --purge verrà utilizzata.",identity.publisher),
        if action == "remove" {"La rimozione elimina le revisioni e i dati dell'app. Snapd normalmente crea uno snapshot automatico, salvo disattivazione nelle impostazioni di sistema. Esegui un backup dei dati che desideri conservare prima di confermare.".into()}
            else {"Snapd può installare basi, runtime e componenti necessari all'app e gestisce connessioni e aggiornamenti automatici secondo la configurazione di Ubuntu. L'operazione seleziona soltanto questa app e conserva il canale stable approvato.".into()},
        "La conferma scade dopo 2 minuti ed è monouso. Identità, publisher, revisione installata e canale vengono ricontrollati prima dell'autenticazione amministratore.".into()];
    Ok((Invocation {program:"/usr/bin/pkexec".into(),args},Proof::Snap {action:action.into(),approved},details,true))
}

/// Start an installed app, detached from this process. The identifier is validated and no shell is
/// involved: a Flatpak or Snap by name, an APT package through its visible launcher file.
pub fn open_app(package: Package) -> Result<()> {
    ensure_ubuntu()?;
    let (program, args): (&str, Vec<String>) = match package.source {
        Source::Flatpak => {
            flatpak_id(&package.id)?;
            let id = app_id(&package.id).to_string();
            if !available("/usr/bin/flatpak") { return Err("Flatpak non è installato.".into()); }
            if !flatpak_installed()?.iter().any(|p| app_id(&p.id) == id) { return Err("Questa app Flatpak non risulta installata.".into()); }
            ("/usr/bin/flatpak", vec!["run".into(), id])
        },
        Source::Snap => {
            snap_name(&package.id)?;
            if !available("/usr/bin/snap") { return Err("Snap non è installato.".into()); }
            if !snap_local_records()?.iter().any(|v| v["name"].as_str() == Some(&package.id)) { return Err("Questo Snap non risulta installato.".into()); }
            let launcher = crate::catalog::snap_desktop_file(&package.id).ok_or("Questa app non ha un'interfaccia grafica da avviare: si usa dal terminale.")?;
            if !available("/usr/bin/gio") { return Err("Lo strumento gio non è disponibile.".into()); }
            ("/usr/bin/gio", vec!["launch".into(), launcher.to_string_lossy().into_owned()])
        },
        Source::Apt => {
            apt_id(&package.id)?;
            if !apt_installed()?.iter().any(|p| base_id(&p.id) == base_id(&package.id)) { return Err("Questo pacchetto non risulta installato.".into()); }
            let launcher = crate::catalog::apt_desktop_file(base_id(&package.id)).ok_or("Questo pacchetto non ha un'app da avviare.")?;
            if !available("/usr/bin/gio") { return Err("Lo strumento gio non è disponibile.".into()); }
            ("/usr/bin/gio", vec!["launch".into(), launcher.to_string_lossy().into_owned()])
        },
    };
    use std::os::unix::process::CommandExt;
    let mut child = Command::new(program).args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0)
        .spawn().map_err(|e| format!("Impossibile avviare l'app: {e}"))?;
    // Reap it when it ends so no zombie is left behind; the app itself keeps running on its own.
    thread::spawn(move || { let _ = child.wait(); });
    Ok(())
}

pub fn search_packages(query_text: String, source: String) -> Result<Vec<Package>> {
    ensure_ubuntu()?;
    let q = validate_query(&query_text)?;
    let (apt, flatpak, snap) = selected(&source)?;
    let mut result = Vec::new();
    if apt { result.extend(apt_search(q)?); }
    if flatpak && available("/usr/bin/flatpak") {
        match flatpak_search(q) {Ok(packages)=>result.extend(packages),Err(_) if source == "all"=>{},Err(e)=>return Err(e)}
    }
    if source == "flatpak" && !available("/usr/bin/flatpak") { return Err("Flatpak non è installato.".into()); }
    if snap && available("/usr/bin/snap") {
        match snap_search(q) {Ok(packages)=>result.extend(packages),Err(_) if source == "all"=>{},Err(e)=>return Err(e)}
    }
    if source == "snap" && !available("/usr/bin/snap") { return Err("Snap non è installato.".into()); }
    enrich_packages(&mut result);
    Ok(result)
}
pub fn list_installed(source: String) -> Result<Vec<Package>> {
    ensure_ubuntu()?;
    let (apt,flatpak,snap) = selected(&source)?;
    let mut result = Vec::new();
    if apt { result.extend(apt_installed()?); }
    if flatpak && available("/usr/bin/flatpak") {
        match flatpak_installed() {Ok(packages)=>result.extend(packages),Err(_) if source == "all"=>{},Err(e)=>return Err(e)}
    }
    if source == "flatpak" && !available("/usr/bin/flatpak") { return Err("Flatpak non è installato.".into()); }
    if snap && available("/usr/bin/snap") {
        match snap_installed() {Ok(packages)=>result.extend(packages),Err(_) if source == "all"=>{},Err(e)=>return Err(e)}
    }
    if source == "snap" && !available("/usr/bin/snap") { return Err("Snap non è installato.".into()); }
    result.sort_by_key(|p| p.name.to_lowercase());
    enrich_packages(&mut result);
    Ok(result)
}
pub fn list_updates(source: String) -> Result<Vec<Package>> {
    ensure_ubuntu()?;
    let (apt,flatpak,snap) = selected(&source)?;
    let mut result = Vec::new();
    if apt {
        let installed: HashMap<_,_> = apt_installed()?.into_iter().map(|p| (p.id.clone(),p)).collect();
        for mut p in apt_updates()? {
            if let Some(old) = installed.get(&p.id) { p.summary = old.summary.clone(); }
            result.push(p);
        }
    }
    if flatpak && available("/usr/bin/flatpak") {
        match flatpak_updates() {Ok(packages)=>result.extend(packages),Err(_) if source == "all"=>{},Err(e)=>return Err(e)}
    }
    if source == "flatpak" && !available("/usr/bin/flatpak") { return Err("Flatpak non è installato.".into()); }
    if snap && available("/usr/bin/snap") {
        match snap_updates() {Ok(packages)=>result.extend(packages),Err(_) if source == "all"=>{},Err(e)=>return Err(e)}
    }
    if source == "snap" && !available("/usr/bin/snap") { return Err("Snap non è installato.".into()); }
    enrich_packages(&mut result);
    Ok(result)
}

pub(crate) fn enrich_packages(packages: &mut [Package]) {
    let flatpak = if packages.iter().any(|p| p.source == Source::Flatpak) { Some(crate::catalog::flatpak_apps()) } else { None };
    let flatpak_index: HashMap<&str, &crate::catalog::CatalogApp> = flatpak.as_ref()
        .map(|apps| apps.iter().map(|app| (app.id.as_str(), app)).collect()).unwrap_or_default();
    for package in packages {
        let (source,id)=match package.source {
            Source::Apt=>{
                let base=base_id(&package.id).to_string();
                if let Some(meta)=crate::catalog::apt_meta(&base) {
                    // Show the application's own name and summary, not the package name.
                    package.kind="application".into();
                    if !meta.name.is_empty() { package.name=meta.name.clone(); }
                    if !meta.summary.is_empty() { package.summary=meta.summary.clone(); }
                    if !meta.id.is_empty() { package.app_id=Some(crate::categories::id_key(&meta.id)); }
                    package.categories=crate::categories::from_freedesktop_list(&meta.categories).into_iter().map(String::from).collect();
                } else {
                    package.kind=if crate::catalog::apt_is_application(&base) {"application".into()} else {"package".into()};
                }
                ("apt",base_id(&package.id))
            },
            Source::Flatpak=>{
                package.kind="application".into();
                let application=app_id(&package.id);
                package.app_id=Some(crate::categories::id_key(application));
                if let Some(entry)=flatpak_index.get(application) {
                    package.categories=entry.categories.iter().map(|c|c.to_string()).collect();
                }
                ("flatpak",application)
            },
            // Snap's canonical type and protected-component classification were
            // already obtained from snapd, not inferred from a display label.
            Source::Snap=>("snap",package.id.as_str()),
        };
        if package.icon_url.is_none() {
            package.icon_url=crate::catalog::local_icon(source,id,package.scope.as_deref(),package.remote.as_deref());
        }
    }
}

fn candidate_version(id: &str) -> Result<String> {
    apt_id(id)?;
    let output = query("/usr/bin/apt-cache", &["policy", "--", id])?;
    let version = output.lines().find_map(|l| l.trim().strip_prefix("Candidate: "))
        .filter(|s| *s != "(none)").ok_or("Il pacchetto non ha una versione candidata APT.")?;
    if version.is_empty() || version.len() > 255 || !version.bytes().all(|b| b.is_ascii_alphanumeric() || b".:+~-".contains(&b)) {
        return Err("Versione candidata APT non valida.".into());
    }
    Ok(version.into())
}
fn apt_details(package: Package) -> Result<PackageDetails> {
    apt_id(&package.id)?;
    let text = query("/usr/bin/apt-cache", &["show", "--no-all-versions", "--", &package.id])?;
    let record = parse_control(&text).into_iter().next().ok_or("Metadati del pacchetto APT non disponibili.")?;
    let description = record.get("Description-en").or_else(|| record.get("Description")).cloned().unwrap_or_default();
    // The first line is the one-line synopsis, already shown as the summary: keep only the long text.
    let long_description = description.split_once('\n').map(|(_, rest)| rest.trim().to_string()).filter(|rest| !rest.is_empty());
    let installed = apt_installed()?.into_iter().find(|p| p.id == package.id || base_id(&p.id) == base_id(&package.id));
    let update_available = apt_updates()?.iter().any(|p| base_id(&p.id) == base_id(&package.id));
    let p = Package { id: package.id.clone(), name: package.id, summary: description.lines().next().unwrap_or("").into(),
        version: record.get("Version").cloned().unwrap_or_default(), source: Source::Apt, installed: installed.is_some(),
        update_available, remote: None, scope: None, publisher: None, channel: None, confinement: None, kind:default_package_kind(),icon_url:None,app_id:None,categories:Vec::new() };
    Ok(PackageDetails { package: p, description: long_description.unwrap_or(description), homepage: record.get("Homepage").cloned(),
        size: record.get("Installed-Size").map(|s| format!("{s} KiB installati")), license: None })
}
fn flatpak_info_command(package: &Package, metadata: Option<&str>) -> Result<Invocation> {
    flatpak_id(&package.id)?;
    let scope = scope_arg(package.scope.as_deref())?;
    let installed = flatpak_installed()?.into_iter().any(|p| p.scope == package.scope && (p.id == package.id || app_id(&p.id) == package.id));
    let mut args: Vec<String> = if installed { vec!["info".into(), scope.into()] } else {
        let remote = package.remote.as_deref().ok_or("Repository Flatpak non specificato.")?;
        remote_id(remote)?;
        vec!["remote-info".into(),scope.into(),remote.into()]
    };
    if let Some(flag) = metadata { args.push(flag.into()); }
    args.push(package.id.clone());
    Ok(Invocation { program: "/usr/bin/flatpak".into(), args })
}
fn run_checked(inv: &Invocation) -> Result<String> {
    let output = run(inv, QUERY_TIMEOUT, false)?;
    if output.text.contains("[output troncato a 2 MiB]") { return Err("Risposta incompleta: il comando supera il limite di sicurezza. Operazione annullata.".into()); }
    if output.success { Ok(output.text) } else { Err(output.text.trim().into()) }
}
fn flatpak_details(package: Package) -> Result<PackageDetails> {
    let text = run_checked(&flatpak_info_command(&package,None)?)?;
    let fields: HashMap<_,_> = text.lines().filter_map(|l| l.trim().split_once(':'))
        .map(|(k,v)| (k,v.trim())).collect();
    let mut package = package;
    let installed = flatpak_installed()?;
    if let Some(p) = installed.into_iter().find(|p| p.scope == package.scope && (p.id == package.id || app_id(&p.id) == package.id)) { package = p; }
    if package.installed {
        package.update_available = flatpak_updates()?.iter().any(|p| p.id == package.id && p.scope == package.scope);
    }
    if let Some(version) = fields.get("Version") { package.version = (*version).into(); }
    // Show the app's own description, not the raw `flatpak info` output.
    let description = crate::catalog::flatpak_description(app_id(&package.id), package.scope.as_deref(), package.remote.as_deref())
        .filter(|text| !text.is_empty()).unwrap_or_else(|| package.summary.clone());
    Ok(PackageDetails { package, description, homepage: None,
        size: fields.get("Installed").or_else(|| fields.get("Download")).map(|s| s.to_string()),
        license: fields.get("License").map(|s| s.to_string()) })
}
pub fn package_details(package: Package) -> Result<PackageDetails> {
    ensure_ubuntu()?;
    let mut details=match package.source { Source::Apt => apt_details(package), Source::Flatpak => flatpak_details(package), Source::Snap => snap_details(package) }?;
    enrich_packages(std::slice::from_mut(&mut details.package));
    Ok(details)
}

#[derive(Debug, PartialEq, Eq)]
struct AptEffects { lines: Vec<String>, removals: Vec<String>, installs: Vec<String> }
fn apt_effects(text: &str) -> Result<AptEffects> {
    let mut effects = AptEffects { lines: Vec::new(), removals: Vec::new(), installs: Vec::new() };
    for line in text.lines() {
        let words: Vec<_> = line.split_whitespace().collect();
        if let Some(kind @ ("Inst" | "Remv" | "Conf")) = words.first().copied() {
            let id = words.get(1).ok_or("Simulazione APT non riconosciuta.")?;
            apt_id(id)?;
            if kind == "Remv" { effects.removals.push((*id).into()); }
            if kind == "Inst" { effects.installs.push((*id).into()); }
            effects.lines.push(line.to_string());
        }
    }
    Ok(effects)
}
fn critical_package(id: &str) -> bool {
    let id = base_id(id);
    matches!(id, "apt" | "dpkg" | "bash" | "dash" | "libc6" | "sudo" | "pkexec" | "polkitd" |
        "policykit-1" | "coreutils" | "mount" | "util-linux" | "login" | "passwd" | "init" |
        "network-manager" | "gdm3" | "gnome-shell" | "dbus" | "openssh-server") ||
        ["systemd", "ubuntu-minimal", "ubuntu-standard", "ubuntu-desktop", "linux-image", "linux-generic", "linux-virtual", "linux-oem", "grub", "shim", "cryptsetup", "base-files", "base-passwd"].iter().any(|prefix| id.starts_with(prefix))
}
fn check_removals(effects: &AptEffects, target: &str, action: &str) -> Result<()> {
    if action != "remove" && !effects.removals.is_empty() {
        return Err("Operazione bloccata: l'installazione o l'aggiornamento rimuoverebbe altri pacchetti.".into());
    }
    if action == "remove" {
        if effects.removals.len() > 20 { return Err("Rimozione bloccata: oltre 20 pacchetti coinvolti. Usa gli strumenti di sistema per una revisione approfondita.".into()); }
        if !effects.installs.is_empty() { return Err("Rimozione bloccata: APT installerebbe altri pacchetti.".into()); }
        if !effects.removals.iter().any(|id| base_id(id) == base_id(target)) {
            return Err("Il pacchetto richiesto non risulta rimuovibile o installato.".into());
        }
    }
    for id in &effects.removals {
        if critical_package(id) { return Err(format!("Rimozione bloccata: {id} è un componente critico del sistema.")); }
        let flags = query("/usr/bin/dpkg-query", &["-W", "-f=${Essential}\t${Protected}\t${Priority}\n", "--", id])?;
        for line in flags.lines() {
            let c: Vec<_> = line.split('\t').collect();
            if c.first() == Some(&"yes") || c.get(1) == Some(&"yes") || c.get(2).map(|x| matches!(*x,"required"|"important")).unwrap_or(false) {
                return Err(format!("Rimozione bloccata: {id} è Essential, Protected o di priorità elevata."));
            }
        }
    }
    Ok(())
}

enum Proof { Apt { simulation: Invocation, effects: AptEffects, target: String, action: String },
    Flatpak { checks: Vec<(Invocation,String)>, remotes: Vec<Remote> }, Snap {action:String,approved:SnapApproval}, None }
/// What a confirmed plan does, in the words the notifications use.
#[derive(Debug, Clone)]
pub struct OperationInfo { pub action: String, pub label: String, pub source: String }
struct Pending { created: Instant, invocation: Invocation, proof: Proof, info: OperationInfo }
/// The operation being executed right now, if any (set only while `MUTATING` is held).
static CURRENT: Mutex<Option<OperationInfo>> = Mutex::new(None);

/// Description of a prepared plan, read without consuming it.
pub fn operation_info(token: &str) -> Option<OperationInfo> {
    PLANS.get()?.lock().ok()?.get(token).map(|pending| pending.info.clone())
}
/// True while a package operation runs, whether or not the window is visible.
pub fn operation_running() -> bool { MUTATING.load(Ordering::Acquire) }
pub fn running_operation() -> Option<OperationInfo> { CURRENT.lock().ok().and_then(|current| current.clone()) }

fn apt_plan(package: Option<Package>, action: &str) -> Result<(Invocation, Proof, Vec<String>, bool)> {
    if !available("/usr/bin/pkexec") { return Err("pkexec non disponibile: installa pkexec con gli strumenti di Ubuntu.".into()); }
    let mut common = vec!["-y".to_string(), "-o".into(), "DPkg::Lock::Timeout=30".into(),
        "-o".into(), "Acquire::Retries=1".into(), "-o".into(), "Acquire::http::Timeout=30".into(),
        "-o".into(), "Acquire::https::Timeout=30".into(), "-o".into(), "APT::Get::AutomaticRemove=false".into(),
        "-o".into(), "Dpkg::Options::=--force-confdef".into(), "-o".into(), "Dpkg::Options::=--force-confold".into()];
    if action == "refresh" {
        common.push("update".into());
        let mut args = vec!["/usr/bin/apt-get".into()]; args.extend(common);
        return Ok((Invocation { program: "/usr/bin/pkexec".into(), args }, Proof::None,
            vec!["Aggiorna soltanto gli indici APT dei repository già configurati. Nessun pacchetto verrà aggiornato.".into(),
                "L'autenticazione amministratore verrà richiesta da PolicyKit.".into()],true));
    }
    let package = package.ok_or("Seleziona un pacchetto per questa operazione.")?;
    if package.source != Source::Apt { return Err("Sorgente del pacchetto non coerente.".into()); }
    apt_id(&package.id)?;
    let installed = apt_installed()?.iter().any(|p| p.id == package.id || base_id(&p.id) == base_id(&package.id));
    if action == "remove" && critical_package(&package.id) { return Err("Rimozione bloccata: componente critico di Ubuntu.".into()); }
    if (action == "remove" || action == "update") && !installed { return Err("Il pacchetto non è installato.".into()); }
    if action == "install" && installed { return Err("Il pacchetto è già installato; usa Aggiorna se disponibile.".into()); }
    if action == "update" && !apt_updates()?.iter().any(|p| base_id(&p.id) == base_id(&package.id)) {
        return Err("Nessun aggiornamento disponibile per questo pacchetto.".into());
    }
    let mut operation = Vec::<String>::new();
    if action != "remove" { operation.push("--no-remove".into()); }
    if action == "update" { operation.push("--only-upgrade".into()); }
    operation.push(if action == "remove" {"remove"} else {"install"}.into());
    operation.push("--".into());
    operation.push(if action == "remove" { format!("{}-", package.id) } else { format!("{}={}",package.id,candidate_version(&package.id)?) });
    let mut simulation_args = vec!["--simulate".into()]; simulation_args.extend(operation.clone());
    let simulation = Invocation { program: "/usr/bin/apt-get".into(), args: simulation_args };
    let output = run_checked(&simulation)?;
    let effects = apt_effects(&output)?;
    check_removals(&effects,&package.id,action)?;
    if action != "remove" && effects.installs.is_empty() { return Err("Non risultano modifiche da applicare a questo pacchetto.".into()); }
    common.extend(operation);
    let mut args = vec!["/usr/bin/apt-get".into()]; args.extend(common);
    let details = vec![format!("Revisione APT: {} installazioni/aggiornamenti e {} rimozioni. Le dipendenze elencate sono incluse nel consenso.", effects.installs.len(),effects.removals.len()),
        output, "Il piano scade dopo 2 minuti ed è utilizzabile una sola volta. APT verrà simulato di nuovo prima dell'esecuzione.".into(),
        "Le configurazioni locali vengono conservate; nessun autoremove, purge o aggiornamento globale.".into()];
    Ok((Invocation { program: "/usr/bin/pkexec".into(), args },
        Proof::Apt { simulation, effects, target: package.id, action: action.into() },details,true))
}

fn flatpak_remote(package: &Package, install: bool) -> Result<Remote> {
    let name = package.remote.as_deref().ok_or("Repository Flatpak non specificato.")?;
    remote_id(name)?;
    let scope = if install { "user" } else { package.scope.as_deref().unwrap_or("user") };
    scope_arg(Some(scope))?;
    let remote = remotes()?.into_iter().find(|r| r.name == name && r.scope == scope)
        .ok_or_else(|| format!("Il repository {name} non è configurato nell'ambito {scope}. Nexa Store non aggiunge repository automaticamente."))?;
    if !remote.verified { return Err("Operazione bloccata: il repository Flatpak non verifica le firme GPG.".into()); }
    Ok(remote)
}
fn flatpak_plan(package: Option<Package>, action: &str) -> Result<(Invocation, Proof, Vec<String>, bool)> {
    if !available("/usr/bin/flatpak") { return Err("Flatpak non è installato.".into()); }
    if action == "refresh" {
        let rows = remotes()?;
        if !rows.iter().any(|r| r.scope == "user") { return Err("Nessun repository Flatpak utente configurato.".into()); }
        if rows.iter().any(|r| r.scope == "user" && !r.verified) { return Err("Un repository utente non verifica le firme GPG; aggiornamento metadati bloccato.".into()); }
        return Ok((Invocation::new("/usr/bin/flatpak", &["update", "--user", "--appstream", "--noninteractive", "--assumeyes"]),
            Proof::Flatpak {checks:vec![],remotes:rows},vec!["Aggiorna soltanto i metadati AppStream dei repository Flatpak utente configurati; le app restano invariate.".into()],false));
    }
    let mut package = package.ok_or("Seleziona un'app Flatpak.")?;
    if package.source != Source::Flatpak { return Err("Sorgente del pacchetto non coerente.".into()); }
    flatpak_id(&package.id)?;
    let requested_scope = scope_arg(package.scope.as_deref())?;
    let installed = flatpak_installed()?.into_iter().find(|p| (p.id == package.id || app_id(&p.id) == package.id)
        && scope_arg(p.scope.as_deref()).ok() == Some(requested_scope));
    if action == "install" && installed.is_some() { return Err("L'app è già installata in questo ambito.".into()); }
    if action != "install" { package = installed.ok_or("App Flatpak non installata nell'ambito selezionato.")?; }
    let mut checks = Vec::new();
    let mut details = Vec::new();
    let trusted_remotes = remotes()?;
    let mut approved_commit = None;
    let scope = if action == "install" { "--user" } else { scope_arg(package.scope.as_deref())? };
    let reference;
    if action == "install" || action == "update" {
        let remote = flatpak_remote(&package, action == "install")?;
        let remote_scope = scope_arg(Some(&remote.scope))?;
        let resolve = Invocation::new("/usr/bin/flatpak", &["remote-info", remote_scope, "--show-ref", &remote.name, &package.id]);
        reference = run_checked(&resolve)?.trim().to_string();
        flatpak_id(&reference)?;
        if !reference.starts_with("app/") { return Err("È consentita soltanto la gestione di app Flatpak, non dei runtime direttamente.".into()); }
        checks.push((resolve,reference.clone()));
        let commit = Invocation::new("/usr/bin/flatpak", &["remote-info", remote_scope, "--show-commit", &remote.name, &reference]);
        let value = run_checked(&commit)?.trim().to_string();
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("Commit Flatpak non valido.".into()); }
        checks.push((commit,value.clone()));
        approved_commit = Some(value.clone());
        details.push(format!("Repository esistente con verifica GPG: {} ({}, {}). Riferimento: {}. Commit: {}.",remote.name,remote.scope,remote.url,reference,value));
        let metadata = query("/usr/bin/flatpak", &["remote-info", remote_scope, "--show-metadata", &remote.name, &reference])?;
        details.push(format!("Metadati e permessi dichiarati dall'app:\n{}", metadata.chars().take(16000).collect::<String>()));
        package.remote = Some(remote.name);
    } else { reference = package.id.clone(); }
    if action != "install" {
        let installed_commit = Invocation::new("/usr/bin/flatpak", &["info", scope, "--show-commit", &reference]);
        let value = run_checked(&installed_commit)?.trim().to_string();
        checks.push((installed_commit,value));
    }
    let mut args = vec![match action { "install" => "install", "remove" => "uninstall", "update" => "update", _ => return Err("Azione non valida.".into()) }.into(),
        scope.into(), "--noninteractive".into(), "--assumeyes".into()];
    if action == "update" {
        args.push(format!("--commit={}",approved_commit.ok_or("Commit approvato mancante.")?));
    }
    if action == "install" { args.push(package.remote.ok_or("Repository mancante.")?); }
    args.push(reference);
    details.push(if action == "remove" {"Verrà rimossa soltanto l'app selezionata. I dati personali vengono conservati; nessuna rimozione dei runtime inutilizzati.".into()}
        else {"Flatpak può includere runtime e dipendenze richiesti dall'app. Il relativo spazio e download vengono risolti da Flatpak all'esecuzione; nessun nuovo repository verrà aggiunto.".into()});
    details.push("Conferma valida per 2 minuti e per una sola esecuzione. I riferimenti e i commit vengono ricontrollati prima di procedere.".into());
    Ok((Invocation { program:"/usr/bin/flatpak".into(),args }, Proof::Flatpak {checks,remotes:trusted_remotes},details,scope == "--system"))
}

pub fn prepare_operation(package: Option<Package>, action: String, source: Option<String>) -> Result<Plan> {
    ensure_ubuntu()?;
    if MUTATING.load(Ordering::Acquire) { return Err("Un'altra operazione è in corso. Attendi il completamento.".into()); }
    if !matches!(action.as_str(),"install"|"remove"|"update"|"refresh") { return Err("Azione non valida.".into()); }
    let selected_source = match (&package,source.as_deref()) {
        (Some(p),None) => p.source,
        (Some(p),Some("apt")) if p.source == Source::Apt => Source::Apt,
        (Some(p),Some("flatpak")) if p.source == Source::Flatpak => Source::Flatpak,
        (None,Some("apt")) => Source::Apt,
        (None,Some("flatpak")) => Source::Flatpak,
        (Some(p),Some("snap")) if p.source == Source::Snap => Source::Snap,
        (None,Some("snap")) => Source::Snap,
        _ => return Err("Seleziona una singola sorgente valida per l'operazione.".into()),
    };
    if action == "refresh" && package.is_some() { return Err("L'aggiornamento indici non richiede un pacchetto.".into()); }
    let label = package.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| match selected_source {Source::Apt=>"APT",Source::Flatpak=>"Flatpak",Source::Snap=>"Snap"}.into());
    let (invocation,proof,details,requires_admin) = match selected_source {
        Source::Apt => apt_plan(package,&action)?, Source::Flatpak => flatpak_plan(package,&action)?, Source::Snap => snap_plan(package,&action)?,
    };
    let mut bytes = [0u8;32];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("Impossibile generare un token sicuro: {e}"))?;
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let command = invocation.display();
    let mut plans = PLANS.get_or_init(|| Mutex::new(HashMap::new())).lock().map_err(|_|"Archivio piani non disponibile.")?;
    plans.retain(|_,p| p.created.elapsed() < PLAN_TTL);
    if plans.len() >= 20 { plans.clear(); }
    plans.insert(token.clone(),Pending {created:Instant::now(),invocation,proof,info:OperationInfo {action:action.clone(),label:label.clone(),source:match selected_source {Source::Apt=>"apt",Source::Flatpak=>"flatpak",Source::Snap=>"snap"}.into()}});
    let verb = match action.as_str() {"install"=>"Installa","remove"=>"Rimuovi","update"=>"Aggiorna",_=>"Aggiorna indici"};
    Ok(Plan {token,title:format!("{verb} {label}"),summary:if requires_admin {"Richiede conferma e autenticazione amministratore quando richiesta dal sistema.".into()}
        else {"Richiede la tua conferma prima di modificare il sistema.".into()},command,details,requires_admin})
}

pub fn execute_operation(token: String) -> Result<OperationResult> {
    ensure_ubuntu()?;
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("Token di conferma non valido.".into()); }
    if MUTATING.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err() {
        return Err("Un'altra operazione è già in corso.".into());
    }
    // This gate spans revalidation and execution. A token is consumed even if the
    // operation fails, so retries always require a new human-reviewed plan.
    let result = (|| {
        let plan = PLANS.get_or_init(||Mutex::new(HashMap::new())).lock().map_err(|_|"Archivio piani non disponibile.")?
            .remove(&token).ok_or("Conferma mancante o già utilizzata. Prepara un nuovo piano.")?;
        if plan.created.elapsed() >= PLAN_TTL { return Err("Il piano è scaduto. Riapri la conferma per generarne uno nuovo.".into()); }
        if let Ok(mut current) = CURRENT.lock() { *current = Some(plan.info.clone()); }
        match &plan.proof {
            Proof::Apt {simulation,effects,target,action} => {
                let current = apt_effects(&run_checked(simulation)?)?;
                check_removals(&current,target,action)?;
                if &current != effects { return Err("Lo stato APT è cambiato. Rivedi un nuovo piano prima di procedere.".into()); }
            },
            Proof::Flatpak {checks,remotes:approved_remotes} => {
                if &remotes()? != approved_remotes { return Err("Repository Flatpak modificati (ambito, URL o verifica GPG). Prepara un nuovo piano.".into()); }
                for (inv,expected) in checks {
                    if run_checked(inv)?.trim() != expected { return Err("I metadati o lo stato Flatpak sono cambiati. Prepara un nuovo piano.".into()); }
                }
            },
            Proof::Snap {action,approved} => {
                if &snap_approval(&approved.name,action,&approved.channel)? != approved {
                    return Err("Identità, canale o revisione Snap sono cambiati. Prepara un nuovo piano prima di procedere.".into());
                }
            },
            Proof::None => {},
        }
        let result = run(&plan.invocation,MUTATION_TIMEOUT,true)?;
        Ok(OperationResult {success:result.success,output:if result.text.trim().is_empty() {
            if result.success {"Operazione completata.".into()} else {"Operazione annullata o non riuscita. Nessun dettaglio restituito dal gestore pacchetti.".into()}
        } else {result.text}})
    })();
    // In the exceptionally rare unkillable-process case, the monitor in run()
    // owns this gate until actual process termination.
    if let Ok(mut current) = CURRENT.lock() { *current = None; }
    let held = result.as_ref().err().map(|e: &String| e.contains("Il processo è ancora attivo")).unwrap_or(false);
    if !held { MUTATING.store(false,Ordering::Release); }
    result
}

pub fn smoke_test() -> Result<serde_json::Value> {
    let system = system_info()?;
    let snap_verification=if system.snap_available {Some(snap_smoke_test()?)} else {None};
    let found = search_packages("gimp".into(),"apt".into())?;
    let installed = list_installed("all".into())?;
    let updates = list_updates("all".into())?;
    let gimp = found.iter().find(|p|p.id == "gimp").cloned().ok_or("Ricerca reale gimp senza risultati.")?;
    let details = package_details(gimp.clone())?;
    let (plan_action,plan_package) = if gimp.installed {("remove",gimp.clone())} else {("install",gimp.clone())};
    let plan = prepare_operation(Some(plan_package),plan_action.into(),None)?;
    let mut invalid = gimp.clone(); invalid.id = "gimp;touch /tmp/nexa-unsafe".into();
    let rejects_invalid = prepare_operation(Some(invalid),"install".into(),None).is_err();
    let bash = Package {id:"bash".into(),name:"bash".into(),summary:String::new(),version:String::new(),source:Source::Apt,
        installed:true,update_available:false,remote:None,scope:None,publisher:None,channel:None,confinement:None,kind:default_package_kind(),icon_url:None,app_id:None,categories:Vec::new()};
    let rejects_essential = prepare_operation(Some(bash),"remove".into(),None).is_err();
    let rejects_token = execute_operation("invalid-token".into()).is_err();
    if !rejects_invalid || !rejects_essential || !rejects_token { return Err("Una verifica di sicurezza non è riuscita.".into()); }
    Ok(serde_json::json!({"ok":true,"system":system,"searchCount":found.len(),"searchSample":found.iter().take(4).collect::<Vec<_>>(),
        "installedCount":installed.len(),"updatesCount":updates.len(),"updates":updates,"details":details,
        "dryRun":{"title":plan.title,"command":plan.command,"details":plan.details},
        "safety":{"invalidIdentifierRejected":rejects_invalid,"essentialRemovalRejected":rejects_essential,"invalidTokenRejected":rejects_token},
        "snapVerification":snap_verification,"mutationsExecuted":false}))
}

fn snap_smoke_test() -> Result<serde_json::Value> {
    let found=search_packages("gimp".into(),"snap".into())?;
    let installed=list_installed("snap".into())?;
    let updates=list_updates("snap".into())?;
    let gimp=found.iter().find(|p|p.id == "gimp").cloned().ok_or("Ricerca Snap reale gimp senza risultati.")?;
    let details=package_details(gimp)?;
    let hello=snap_package(&snap_store_record("hello-world")?,false,false)?;
    let install_plan=prepare_operation(Some(hello.clone()),"install".into(),None)?;
    let classic=snap_package(&snap_store_record("code")?,false,false)?;
    let classic_rejected=prepare_operation(Some(classic),"install".into(),None).is_err();
    let critical=installed.iter().find(|p|p.id == "core24" || p.id == "snapd").cloned().ok_or("Nessun componente Snap critico disponibile per la prova.")?;
    let critical_rejected=prepare_operation(Some(critical),"remove".into(),None).is_err();
    let mut invalid=hello; invalid.id="--classic".into();
    let invalid_rejected=prepare_operation(Some(invalid),"install".into(),None).is_err();
    if !classic_rejected || !critical_rejected || !invalid_rejected {return Err("Verifica di sicurezza Snap non riuscita.".into());}
    Ok(serde_json::json!({"searchCount":found.len(),"searchSample":found.iter().take(3).collect::<Vec<_>>(),"installedCount":installed.len(),"updatesCount":updates.len(),
        "details":details,"dryRun":{"title":install_plan.title,"command":install_plan.command,"details":install_plan.details},
        "safety":{"classicRejected":classic_rejected,"criticalRemovalRejected":critical_rejected,"invalidNameRejected":invalid_rejected},"mutationsExecuted":false}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snap_fixture() -> serde_json::Value {
        serde_json::json!({"id":"buPKUD3TKqCOgLEjjHx5kSiCpIs5cMuQ","name":"hello-world","title":"Hello World","summary":"Greeting","description":"Test app","version":"6.4","revision":"30",
            "type":"app","status":"available","confinement":"strict","channel":"stable","tracking-channel":"latest/stable","devmode":false,
            "publisher":{"id":"canonical","username":"canonical","display-name":"Canonical","validation":"verified"},
            "channels":{"latest/stable":{"revision":"29","version":"6.4","confinement":"strict"},"latest/edge":{"revision":"30","version":"6.5","confinement":"strict"}}})
    }
    #[test]
    fn snap_names_and_stable_tracks_exclude_options_paths_and_branches() {
        for bad in ["","-snap","--classic","foo;id","foo bar","foo\nbar","foo/bar","foo_instance","foo--bar","12345","foo.snap/../../x","UPPER"] {assert!(snap_name(bad).is_err(),"{bad}");}
        for good in ["hello-world","gimp","0ad","snap-store","core24"] {assert!(snap_name(good).is_ok(),"{good}");}
        assert!(snap_name(&"a".repeat(41)).is_err());
        for good in ["stable","latest/stable","2/stable","5.4/stable"] {assert!(stable_snap_channel(good).is_ok(),"{good}");}
        for bad in ["edge","latest/beta","2/candidate","latest/stable/branch","--classic/stable",""] {assert!(stable_snap_channel(bad).is_err(),"{bad}");}
        assert_eq!(uri_component("gimp &x=--classic"),"gimp%20%26x%3D--classic");
    }
    #[test]
    fn snap_http_parser_accepts_framing_and_rejects_incomplete_data() {
        let body=r#"{"type":"sync","result":[]}"#;
        let fixed=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",body.len());
        assert_eq!(decode_http_response(fixed.as_bytes()).unwrap().1["result"],serde_json::json!([]));
        let chunked=format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",body.len());
        assert!(decode_http_response(chunked.as_bytes()).is_ok());
        assert!(decode_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{}").is_err());
        assert!(decode_http_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nFFFFFF\r\n{}").is_err());
        assert!(decode_http_response(b"garbage").is_err());
    }
    #[test]
    fn snap_identity_uses_approved_stable_revision_and_blocks_unsafe_modes() {
        let fixture=snap_fixture();
        let identity=snap_identity(&fixture,"latest/stable",true).unwrap();
        assert_eq!(identity.revision,"29"); assert_eq!(identity.channel,"latest/stable");
        let package=snap_package(&fixture,false,false).unwrap();
        assert_eq!(package.source,Source::Snap); assert_eq!(package.publisher.as_deref(),Some("Canonical (@canonical) · verificato"));
        assert_eq!(package.kind,"application");
        for mode in ["classic","devmode"] {
            let mut unsafe_snap=fixture.clone();unsafe_snap["channels"]["latest/stable"]["confinement"]=mode.into();
            assert!(snap_identity(&unsafe_snap,"latest/stable",true).is_err());
        }
        let mut dev=fixture.clone();dev["devmode"]=true.into(); assert!(snap_identity(&dev,"latest/stable",true).is_err());
        let mut sideload=fixture.clone();sideload["id"]="".into(); assert!(snap_identity(&sideload,"latest/stable",true).is_err());
        let mut local=fixture;local["revision"]="x1".into(); assert!(snap_identity(&local,"latest/stable",false).is_err());
    }
    #[test]
    fn package_icons_and_categories_use_canonical_metadata_and_safe_urls() {
        let mut fixture=snap_fixture();
        fixture["icon"]="https://dashboard.snapcraft.io/app/icon.png".into();
        assert!(snap_package(&fixture,false,false).unwrap().icon_url.unwrap().starts_with("https://"));
        fixture["name"]="core24".into();fixture["type"]="base".into();
        assert_eq!(snap_package(&fixture,true,false).unwrap().kind,"package");
        for bad in ["javascript:alert(1)","http://example.com/icon.png","data:image/svg+xml,bad","/v2/icons/gimp/icon","https://","https://user@example.com/icon.png","https://example.com/icon\n.png"] {
            fixture["icon"]=bad.into();assert!(snap_icon(&fixture).is_none(),"{bad}");
        }
        let legacy=serde_json::json!({"id":"gimp","name":"gimp","summary":"Editor","version":"3","source":"apt","installed":false,"updateAvailable":false});
        let parsed:Package=serde_json::from_value(legacy).unwrap();
        assert_eq!(parsed.kind,"package");assert!(parsed.icon_url.is_none());
    }
    #[test]
    fn snap_system_components_are_not_managed_as_ordinary_apps() {
        for kind in ["base","kernel","gadget","snapd","os",""] {assert!(critical_snap("example",kind));}
        for name in ["core","core24","bare","snapd","snap-store","firmware-updater","desktop-security-center","prompting-client","snapd-desktop-integration","gnome-46-2404","mesa-2404","gtk-common-themes"] {assert!(critical_snap(name,"app"),"{name}");}
        assert!(!critical_snap("gimp","app"));assert!(!critical_snap("hello-world","app"));assert!(!critical_snap("gnome-calculator","app"));
    }
    #[test]
    fn system_changes_capture_output_from_a_file_and_survive_without_pipes() {
        let result=run(&Invocation::new("/bin/sh",&["-c","echo fuori; echo errore 1>&2; exit 3"]),Duration::from_secs(10),true).unwrap();
        assert!(!result.success);
        assert!(result.text.contains("fuori") && result.text.contains("errore"),"{}",result.text);
        let ok=run(&Invocation::new("/bin/sh",&["-c","printf 'x%.0s' $(seq 1 5000)"]),Duration::from_secs(10),true).unwrap();
        assert!(ok.success && ok.text.len() == 5000);
        // The child writes to a file, so an output that nobody reads cannot stop it with SIGPIPE.
        let long=run(&Invocation::new("/bin/sh",&["-c","i=0; while [ $i -lt 20000 ]; do echo riga $i; i=$((i+1)); done; echo finito"]),Duration::from_secs(20),true).unwrap();
        assert!(long.success && long.text.trim_end().ends_with("finito"));
        let leftovers=fs::read_dir(std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p|p.is_dir()).unwrap_or_else(std::env::temp_dir)).unwrap()
            .filter_map(|e|e.ok()).filter(|e|e.file_name().to_string_lossy().starts_with(&format!("nexa-store-{}-",std::process::id()))).count();
        assert_eq!(leftovers,0,"temporary output files are always deleted");
        assert!(run(&Invocation::new("/nonexistent/program",&[]),Duration::from_secs(5),true).is_err());
    }
    #[test]
    #[ignore = "uses the real pkexec binary (no authorisation is requested for --version)"]
    fn pkexec_runs_with_file_output_and_its_own_process_group() {
        let result=run(&Invocation::new("/usr/bin/pkexec",&["--version"]),Duration::from_secs(10),true).unwrap();
        assert!(result.success,"{}",result.text);
        assert!(result.text.to_lowercase().contains("pkexec"),"{}",result.text);
    }
    #[test]
    fn prepared_operations_can_be_described_without_being_consumed() {
        let info=OperationInfo {action:"install".into(),label:"GIMP".into(),source:"apt".into()};
        let token="c".repeat(64);
        PLANS.get_or_init(||Mutex::new(HashMap::new())).lock().unwrap().insert(token.clone(),Pending {created:Instant::now(),invocation:Invocation::new("/usr/bin/true",&[]),proof:Proof::None,info:info.clone()});
        let read=operation_info(&token).unwrap();
        assert_eq!((read.action.as_str(),read.label.as_str()),("install","GIMP"));
        assert!(operation_info("0".repeat(64).as_str()).is_none());
        PLANS.get().unwrap().lock().unwrap().remove(&token);
    }
    #[test]
    fn identifiers_reject_options_shell_and_version_expressions() {
        for bad in ["", "-bash", "bash;id", "foo bar", "foo\nbar", "bash=1", "bash/stable", "x", "pkg:amd64:extra", "foo$(id)"] { assert!(apt_id(bad).is_err(),"{bad}"); }
        for good in ["bash", "libc6:amd64", "g++", "libstdc++6", "libgtk-4-1", "0ad"] { assert!(apt_id(good).is_ok(),"{good}"); }
        for bad in ["--user", "org.foo/../../x", "runtime/org.foo/x86_64/stable", "org.foo;id", "app/org.foo/x86_64/-stable"] { assert!(flatpak_id(bad).is_err(),"{bad}"); }
        for good in ["org.gimp.GIMP", "app/org.gimp.GIMP/x86_64/stable", "com.example.My_App"] { assert!(flatpak_id(good).is_ok(),"{good}"); }
        assert!(remote_id("--if-not-exists").is_err());
        assert!(remote_id("flathub").is_ok());
        assert!(scope_arg(Some("system;id")).is_err());
    }
    #[test]
    fn installed_parser_excludes_removed_and_half_configured_packages() {
        let rows=parse_dpkg("bash\t5.3\tGNU shell\tii \nold\t1\tOld config\trc \nbroken\t2\tBroken\tiU \n");
        assert_eq!(rows.len(),1); assert_eq!(rows[0].id,"bash");
    }
    #[test]
    fn apt_update_parser_handles_repository_and_version() {
        let rows=parse_apt_updates("Listing...\nchatgpt/stable 26.1007 amd64 [upgradable from: 26.1001]\n");
        assert_eq!(rows.len(),1); assert_eq!(rows[0].version,"26.1007"); assert!(rows[0].update_available);
    }
    #[test]
    fn debian_control_multiline_description_preserves_paragraphs() {
        let rows=parse_control("Package: hello\nVersion: 1.2\nDescription-en: Greeting\n Second line\n .\n Last paragraph\n\nPackage: other\nVersion: 2\n");
        assert_eq!(rows.len(),2); assert_eq!(rows[0]["Description-en"],"Greeting\nSecond line\n\nLast paragraph");
    }
    #[test]
    fn apt_plan_parser_distinguishes_effects_and_blocks_dangerous_changes() {
        let e=apt_effects("Inst gimp (3.2 Ubuntu:26.10 [amd64])\nConf gimp (3.2 Ubuntu:26.10 [amd64])\nRemv bash [5.3]\n").unwrap();
        assert_eq!(e.installs,vec!["gimp"]); assert_eq!(e.removals,vec!["bash"]);
        assert!(check_removals(&e,"gimp","install").is_err());
        assert!(check_removals(&e,"bash","remove").is_err());
        let mut many=apt_effects("").unwrap(); many.removals=(0..21).map(|n|format!("app{n}")).collect();
        assert!(check_removals(&many,"app0","remove").is_err());
        for critical in ["bash","linux-image-6.17-generic","ubuntu-desktop-minimal","systemd-sysv","grub-efi-amd64","libc6:amd64"] {assert!(critical_package(critical));}
        assert!(!critical_package("gimp"));
    }
    #[test]
    fn flatpak_list_in_the_format_printed_by_flatpak_1_18_is_not_dropped() {
        // Real output of `flatpak list --user --app --columns=application,name,description,version,origin,ref`.
        let rows=parse_flatpak_list("com.brave.Browser\tBrave\tFast Internet\t1.97.57\tflathub\tcom.brave.Browser/x86_64/stable\ncom.visualstudio.code\tVisual Studio Code\tCode editing. Redefined.\t1.139.1\tflathub\tcom.visualstudio.code/x86_64/stable\n","user");
        assert_eq!(rows.len(),2,"no installed app may be lost");
        assert_eq!(rows[1].id,"app/com.visualstudio.code/x86_64/stable");
        assert_eq!(app_id(&rows[1].id),"com.visualstudio.code");
        assert_eq!(rows[1].name,"Visual Studio Code");
        assert_eq!(normalize_ref("app/org.gimp.GIMP/x86_64/stable"),"app/org.gimp.GIMP/x86_64/stable");
        assert_eq!(normalize_ref("org.gimp.GIMP/x86_64/stable"),"app/org.gimp.GIMP/x86_64/stable");
        assert!(parse_flatpak_list("x\ty\tz\t1\tflathub\t--evil/x86_64/stable\n","user").is_empty(),"invalid refs stay discarded");
    }
    #[test]
    fn flatpak_parser_preserves_scope_and_full_ref() {
        let rows=parse_flatpak_list("org.gimp.GIMP\tGIMP\tImage editor\t3.2\tflathub\tapp/org.gimp.GIMP/x86_64/stable\n","system");
        assert_eq!(rows.len(),1); assert_eq!(rows[0].scope.as_deref(),Some("system")); assert!(rows[0].id.starts_with("app/"));
    }
    #[test]
    fn searches_are_bounded_and_regex_is_literal() {
        assert!(validate_query("a").is_err()); assert!(validate_query("abc\ndef").is_err()); assert!(validate_query(&"a".repeat(121)).is_err());
        assert_eq!(regex_literal("g++"),"g\\+\\+");
        assert_eq!(regex_literal(".*"),"\\.\\*");
    }
    #[test]
    fn expired_tokens_are_consumed_and_execution_is_one_use() {
        if !ubuntu_info().0 { return; }
        assert!(execute_operation("0".repeat(64)).is_err());
        let expired="e".repeat(64);
        let consumed="d".repeat(64);
        {
            let mut plans=PLANS.get_or_init(||Mutex::new(HashMap::new())).lock().unwrap();
            plans.insert(expired.clone(),Pending {created:Instant::now()-PLAN_TTL-Duration::from_secs(1),
                invocation:Invocation::new("/usr/bin/false",&[]),proof:Proof::None,info:OperationInfo {action:"test".into(),label:"test".into(),source:"apt".into()}});
            plans.insert(consumed.clone(),Pending {created:Instant::now(),
                invocation:Invocation::new("/usr/bin/false",&[]),proof:Proof::None,info:OperationInfo {action:"test".into(),label:"test".into(),source:"apt".into()}});
        }
        let error=execute_operation(expired.clone()).unwrap_err();
        assert!(error.contains("scaduto"));
        assert!(execute_operation(expired).unwrap_err().contains("già utilizzata"));
        // /usr/bin/false is harmless. Even its failed execution consumes consent.
        assert!(!execute_operation(consumed.clone()).unwrap().success);
        assert!(execute_operation(consumed).unwrap_err().contains("già utilizzata"));
        assert!(!MUTATING.load(Ordering::Acquire));
    }
}
