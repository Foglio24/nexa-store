//! Desktop notifications for operations that finish while the window is closed or not in use.
//!
//! Notifications go through the standard `org.freedesktop.Notifications` D-Bus service (GNOME
//! Shell, KDE and others), so nothing is installed besides what the desktop already has. The text
//! is plain: package names come from catalogs, so markup characters are escaped.
use notify_rust::{Hint, Notification, Timeout};
use std::{env, path::PathBuf};

const APP_NAME: &str = "Nexa Store";
/// Same text with an invisible character at the end. GNOME Shell links a notification to an app by
/// its name too (it looks for `<name>.desktop`, and the package ships `Nexa Store.desktop`), and it
/// deletes every notification of a linked app as soon as the sender leaves the bus. This name never
/// matches a file, so a notification sent just before quitting stays until the user dismisses it.
const APP_NAME_DETACHED: &str = "Nexa Store\u{200b}";
const ICON: &str = "nexa-store";
/// Added to the "installed" message of a Flatpak app when the session cannot see its launcher yet.
pub fn menu_hint() -> String { tr!("If it does not show up in the app menu, log out and back in once; meanwhile open it from Nexa Store.") }
const MAX_LABEL: usize = 60;
/// Desktop-entry ids the app can be installed under: the package, the project launcher script and
/// the per-user launcher. The id lets the desktop attribute the notification to the app and open
/// it when the notification is clicked.
const DESKTOP_IDS: [&str; 4] = ["it.nexa.store", "it.nexa.store.local", "nexa-store", "Nexa Store"];

fn data_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    match env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|path| path.is_absolute()) {
        Some(path) => directories.push(path),
        None => if let Some(home) = env::var_os("HOME") { directories.push(PathBuf::from(home).join(".local/share")); },
    }
    let system = env::var("XDG_DATA_DIRS").unwrap_or_default();
    let system = if system.is_empty() { "/usr/local/share:/usr/share".to_string() } else { system };
    directories.extend(system.split(':').filter(|path| path.starts_with('/')).map(PathBuf::from));
    directories
}

/// The first of our desktop entries that exists, without the `.desktop` suffix.
pub fn desktop_entry() -> Option<String> {
    let directories = data_directories();
    DESKTOP_IDS.iter().find(|id| directories.iter().any(|directory| directory.join("applications").join(format!("{id}.desktop")).is_file())).map(|id| id.to_string())
}

fn escape(text: &str) -> String {
    let short: String = text.chars().filter(|c| !c.is_control()).take(MAX_LABEL).collect();
    short.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Title and text announcing that an operation has ended.
pub fn finished_message(action: &str, label: &str, success: bool) -> (String, String) {
    let label = escape(label);
    if action == "check" {
        return if success { (tr!("Update check finished"), tr!("Nexa Store has refreshed the lists and looked for updates.")) }
            else { (tr!("Update check failed"), tr!("Open Nexa Store to see the details.")) };
    }
    if !success {
        let summary = if action == "refresh" { tr!("Check of {} failed", label) } else { tr!("{}: operation failed", label) };
        return (summary, tr!("Open Nexa Store to see the details."));
    }
    match action {
        "install" => (tr!("{} installed", label), tr!("The installation has finished.")),
        "remove" => (tr!("{} removed", label), tr!("The removal has finished.")),
        "update" => (tr!("{} updated", label), tr!("The update has finished.")),
        _ => (tr!("Check of {} finished", label), tr!("Nexa Store has finished updating the lists.")),
    }
}

/// What an update check found, for the end-of-check notification (`None`: the count could not be read).
pub fn updates_found_message(count: Option<usize>) -> String {
    match count {
        Some(0) => tr!("No updates available."),
        Some(1) => tr!("1 update is available."),
        Some(count) => tr!("{} updates are available.", count),
        None => tr!("Open Nexa Store to see the updates."),
    }
}

/// Title and text shown when the window is closed while an operation is still running.
pub fn background_message(action: &str, label: &str) -> (String, String) {
    let label = escape(label);
    let text = match action {
        "install" => tr!("Installing {}. You'll get a notification as soon as it ends.", label),
        "remove" => tr!("Removing {}. You'll get a notification as soon as it ends.", label),
        "update" => tr!("Updating {}. You'll get a notification as soon as it ends.", label),
        "check" => tr!("Checking for updates. You'll get a notification as soon as it ends."),
        _ => tr!("Checking {}. You'll get a notification as soon as it ends.", label),
    };
    (tr!("Nexa Store keeps going in the background"), text)
}

/// How a notification relates to the app that sent it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// Linked to the app: its banner opens the app when clicked, but GNOME Shell deletes it as soon
    /// as the app exits. For notifications sent while the app keeps running.
    Attached,
    /// Not linked to any app: it stays in the notification list after the app has exited, like
    /// `notify-send`. For the final notification of an app that quits right after sending it.
    Detached,
}

fn identity(delivery: Delivery) -> (&'static str, Option<String>) {
    match delivery {
        Delivery::Attached => (APP_NAME, desktop_entry()),
        Delivery::Detached => (APP_NAME_DETACHED, None),
    }
}

/// Show a notification. Returns whether some notification service accepted it; a missing service
/// (for example in a bare session) is not an error for the operation itself. The standard D-Bus
/// route is tried first, then the `notify-send` tool (package libnotify-bin, part of most desktops).
pub fn send(summary: &str, body: &str, delivery: Delivery) -> bool {
    let (name, entry) = identity(delivery);
    let mut notification = Notification::new();
    notification.appname(name).summary(summary).body(body).icon(ICON).timeout(Timeout::Milliseconds(12_000))
        .hint(Hint::Category("transfer.complete".into()));
    if let Some(id) = entry { notification.hint(Hint::DesktopEntry(id)); }
    match notification.show() {
        Ok(_) => true,
        Err(error) => {
            eprintln!("Nexa Store: D-Bus notification failed ({error}); trying notify-send");
            send_with_tool(summary, body, delivery)
        }
    }
}

fn send_with_tool(summary: &str, body: &str, delivery: Delivery) -> bool {
    let (name, entry) = identity(delivery);
    let mut command = std::process::Command::new("notify-send");
    command.args(["--app-name", name, "--icon", ICON, "--expire-time", "12000", "--category", "transfer.complete"]);
    if let Some(id) = entry { command.args(["--hint", &format!("string:desktop-entry:{id}")]); }
    command.args(["--", summary, body]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    command.status().map(|status| status.success()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_describe_each_action_and_failure() {
        assert_eq!(finished_message("install", "GIMP", true).0, "GIMP installed");
        assert_eq!(finished_message("remove", "GIMP", true).0, "GIMP removed");
        assert_eq!(finished_message("update", "GIMP", true).0, "GIMP updated");
        assert_eq!(finished_message("refresh", "APT", true).0, "Check of APT finished");
        let (summary, body) = finished_message("install", "GIMP", false);
        assert_eq!(summary, "GIMP: operation failed");
        assert!(body.contains("Open Nexa Store"));
        assert_eq!(finished_message("refresh", "Snap", false).0, "Check of Snap failed");
        let (title, text) = background_message("install", "GIMP");
        assert_eq!(title, "Nexa Store keeps going in the background");
        assert_eq!(text, "Installing GIMP. You'll get a notification as soon as it ends.");
        assert!(background_message("remove", "x").1.starts_with("Removing x"));
        assert_eq!(finished_message("check", "", true).0, "Update check finished");
        assert_eq!(finished_message("check", "", false).0, "Update check failed");
        assert!(background_message("check", "").1.starts_with("Checking for updates."));
        assert_eq!(updates_found_message(Some(0)), "No updates available.");
        assert_eq!(updates_found_message(Some(1)), "1 update is available.");
        assert_eq!(updates_found_message(Some(5)), "5 updates are available.");
        assert!(updates_found_message(None).contains("Open Nexa Store"));
    }

    #[test]
    fn messages_follow_the_interface_language() {
        crate::i18n::with_language(crate::i18n::Lang::It, || {
            assert_eq!(finished_message("install", "GIMP", true).0, "GIMP installata");
            assert_eq!(finished_message("remove", "GIMP", true).0, "GIMP rimossa");
            assert_eq!(finished_message("update", "GIMP", true).0, "GIMP aggiornata");
            assert_eq!(finished_message("refresh", "APT", true).0, "Controllo APT terminato");
            let (summary, body) = finished_message("install", "GIMP", false);
            assert_eq!(summary, "GIMP: operazione non riuscita");
            assert!(body.contains("Apri Nexa Store"));
            assert_eq!(finished_message("refresh", "Snap", false).0, "Controllo Snap non riuscito");
            let (title, text) = background_message("install", "GIMP");
            assert_eq!(title, "Nexa Store continua in background");
            assert_eq!(text, "Installazione di GIMP in corso: ti avviso appena termina.");
            assert!(background_message("remove", "x").1.starts_with("Rimozione di x"));
            assert_eq!(finished_message("check", "", true).0, "Controllo aggiornamenti terminato");
            assert_eq!(updates_found_message(Some(3)), "Ci sono 3 aggiornamenti disponibili.");
            assert_eq!(updates_found_message(Some(1)), "C'è 1 aggiornamento disponibile.");
        });
    }

    #[test]
    fn a_detached_notification_has_no_app_link_of_any_kind() {
        let (name, entry) = identity(Delivery::Detached);
        assert!(entry.is_none(), "no desktop-entry");
        assert!(name.starts_with("Nexa Store") && name != "Nexa Store", "the name must not match any .desktop file");
        assert_eq!(name.chars().filter(|c| !c.is_whitespace() && *c != '\u{200b}').collect::<String>(), "NexaStore", "it still reads \"Nexa Store\" on screen");
        assert_eq!(identity(Delivery::Attached).0, "Nexa Store");
    }

    #[test]
    fn package_names_cannot_inject_markup_or_flood_the_notification() {
        let (summary, _) = finished_message("install", "<b>evil</b> & \u{7}co", true);
        assert!(!summary.contains('<') && !summary.contains('\u{7}'), "{summary}");
        assert!(summary.contains("&lt;b&gt;evil") && summary.contains("&amp;"));
        let long = "a".repeat(500);
        assert!(finished_message("install", &long, true).0.chars().count() < 90);
    }

    #[test]
    #[ignore = "shows real notifications on the desktop session; run with NEXA_NOTIFY_DELIVERY=attached|detached"]
    fn real_notification_by_delivery_for_the_exit_experiment() {
        let delivery = if std::env::var("NEXA_NOTIFY_DELIVERY").as_deref() == Ok("detached") { Delivery::Detached } else { Delivery::Attached };
        let (summary, body) = finished_message("install", &format!("Test {delivery:?}"), true);
        assert!(send(&summary, &body, delivery));
        // The process ends right here, as the app does after the last notification of a background install.
    }

    #[test]
    #[ignore = "shows a real notification on the desktop session"]
    fn real_notification_is_accepted_by_the_desktop() {
        let (summary, body) = finished_message("install", "Nexa Store test", true);
        println!("desktop entry: {:?}", desktop_entry());
        assert!(send(&summary, &body, Delivery::Attached));
    }
}
