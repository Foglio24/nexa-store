//! Refresh the APT package lists through PackageKit.
//!
//! PackageKit is the service the software updaters of Ubuntu, Debian and their derivatives use (Ubuntu
//! Software Updater, GNOME Software, KDE Discover). Its polkit rule for refreshing the
//! lists (`org.freedesktop.packagekit.system-sources-refresh`) lets a person who is using the
//! computer do it without a password, which is why the Software Updater never asks for one just to
//! look for updates. `pkexec apt-get update`, in contrast, always asks. Nothing here installs,
//! upgrades or removes anything: the only call is `RefreshCache`.
//!
//! Everything is a plain D-Bus call to the system bus; no command is started and no new
//! authorisation rule is added to the system.
use std::{sync::mpsc, thread, time::Duration};
use zbus::{blocking::{Connection, MessageIterator, Proxy}, zvariant::OwnedObjectPath, MatchRule};

const SERVICE: &str = "org.freedesktop.PackageKit";
const ROOT: &str = "/org/freedesktop/PackageKit";
const TRANSACTION: &str = "org.freedesktop.PackageKit.Transaction";
/// `PK_EXIT_ENUM_SUCCESS`.
const EXIT_SUCCESS: u32 = 1;
const REFRESH_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// How a refresh ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The lists were refreshed.
    Done,
    /// PackageKit did its job and it did not work (no network, a repository error…). Asking for a
    /// password would not change that.
    Failed(String),
    /// PackageKit could not be reached or would not let this session do it: the caller may try the
    /// administrator route (`pkexec apt-get update`) instead.
    Refused(String),
}

/// Whether the system has PackageKit, running or started on demand by D-Bus.
pub fn available() -> bool {
    let Ok(connection) = Connection::system() else { return false };
    let Ok(bus) = zbus::blocking::fdo::DBusProxy::new(&connection) else { return false };
    let Ok(name) = zbus::names::BusName::try_from(SERVICE) else { return false };
    bus.name_has_owner(name).unwrap_or(false)
        || bus.list_activatable_names().map(|names| names.iter().any(|activatable| activatable.as_str() == SERVICE)).unwrap_or(false)
}

/// A PackageKit message that says the person may not do this (polkit said no, or nobody could be asked).
fn is_refusal(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["not authorized", "notauthorized", "not authorised", "denied", "authorization", "authorisation", "accessdenied"].iter().any(|needle| lower.contains(needle))
}

enum Event { Error(String), Finished(u32) }

/// Refresh the APT lists and wait until PackageKit says it is done.
pub fn refresh_lists() -> Outcome {
    match run() {
        Ok(Outcome::Failed(text)) if is_refusal(&text) => Outcome::Refused(text),
        Ok(outcome) => outcome,
        // Could not talk to PackageKit at all, or the call itself was refused.
        Err(error) => Outcome::Refused(error.to_string()),
    }
}

fn run() -> zbus::Result<Outcome> {
    let connection = Connection::system()?;
    let service = Proxy::new(&connection, SERVICE, ROOT, "org.freedesktop.PackageKit")?;
    let path: OwnedObjectPath = service.call("CreateTransaction", &())?;
    // Listen before asking, so that no signal can be missed.
    let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal).path(path.as_str())?.interface(TRANSACTION)?.build();
    let signals = MessageIterator::for_match_rule(rule, &connection, Some(64))?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for message in signals {
            let Ok(message) = message else { continue };
            let header = message.header();
            let event = match header.member().map(|member| member.as_str()) {
                Some("ErrorCode") => message.body().deserialize::<(u32, String)>().ok().map(|(_, details)| Event::Error(details)),
                Some("Finished") => message.body().deserialize::<(u32, u32)>().ok().map(|(exit, _)| Event::Finished(exit)),
                _ => None,
            };
            let finished = matches!(event, Some(Event::Finished(_)));
            if let Some(event) = event { if sender.send(event).is_err() { return; } }
            if finished { return; }
        }
    });
    let transaction = Proxy::new(&connection, SERVICE, path.as_str(), TRANSACTION)?;
    transaction.call::<_, _, ()>("SetHints", &(vec!["interactive=true", "background=false"],))?;
    transaction.call::<_, _, ()>("RefreshCache", &(true,))?;
    let mut details: Vec<String> = Vec::new();
    loop {
        match receiver.recv_timeout(REFRESH_TIMEOUT) {
            Ok(Event::Error(text)) => details.push(text),
            Ok(Event::Finished(exit)) if exit == EXIT_SUCCESS => return Ok(Outcome::Done),
            Ok(Event::Finished(exit)) => {
                let text = if details.is_empty() { format!("PackageKit finished with exit code {exit}.") } else { details.join("\n") };
                return Ok(Outcome::Failed(text));
            },
            Err(_) => return Ok(Outcome::Failed(tr!("PackageKit did not answer in time."))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_is_told_apart_from_a_real_failure() {
        for refused in ["Not authorized", "org.freedesktop.DBus.Error.AccessDenied: denied", "Authorization failed", "not authorised to refresh"] {
            assert!(is_refusal(refused), "{refused}");
        }
        for failed in ["Failed to fetch https://example.com/InRelease  Temporary failure resolving 'example.com'", "Could not get lock /var/lib/apt/lists/lock", "PackageKit finished with exit code 2."] {
            assert!(!is_refusal(failed), "{failed}");
        }
    }

    #[test]
    #[ignore = "talks to the real PackageKit and refreshes the APT lists (the same refresh the system's software updater does)"]
    fn real_refresh_needs_no_password() {
        assert!(available(), "PackageKit is not available");
        let started = std::time::Instant::now();
        let outcome = refresh_lists();
        println!("{outcome:?} in {:?}", started.elapsed());
        assert_eq!(outcome, Outcome::Done);
    }
}
