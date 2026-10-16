use tauri::{window::Color, Manager};

#[macro_use]
pub mod i18n;
pub mod appimage;
pub mod backend;
pub mod catalog;
pub mod categories;
pub mod notify;
pub mod packagekit;
pub mod popular;
pub mod store;

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| tr!("Internal operation failed: {}", e))?
}

/// `it` or `en`: the language of the interface, from the system language (see `i18n::detect`).
#[tauri::command]
fn app_language() -> &'static str { i18n::current().code() }
#[tauri::command]
async fn system_info() -> Result<backend::SystemInfo,String> { blocking(backend::system_info).await }
#[tauri::command]
async fn search_packages(query: String, source: String) -> Result<Vec<backend::Package>,String> {
    blocking(move || backend::search_packages(query,source)).await
}
#[tauri::command]
async fn list_installed(source: String) -> Result<Vec<backend::Package>,String> {
    blocking(move || backend::list_installed(source)).await
}
#[tauri::command]
async fn list_updates(source: String) -> Result<Vec<backend::Package>,String> {
    blocking(move || backend::list_updates(source)).await
}
#[tauri::command]
async fn package_details(package: backend::Package) -> Result<backend::PackageDetails,String> {
    blocking(move || backend::package_details(package)).await
}
#[tauri::command]
async fn prepare_operation(package: Option<backend::Package>, action: String, source: Option<String>) -> Result<backend::Plan,String> {
    blocking(move || backend::prepare_operation(package,action,source)).await
}
#[tauri::command]
async fn explore_home(seed: u64) -> Result<store::ExploreHome,String> { blocking(move || store::explore_home(seed)).await }
#[tauri::command]
async fn browse_category(category: String, source: String, sort: String, seed: u64, offset: usize, limit: usize) -> Result<store::CategoryPage,String> {
    blocking(move || store::browse_category(category,source,sort,seed,offset,limit)).await
}
/// Runs a confirmed operation. It never depends on the window: if the window was closed meanwhile it
/// is only hidden, the work goes on, and the user is told when it ends.
#[tauri::command]
async fn open_app(package: backend::Package) -> Result<(),String> { blocking(move || backend::open_app(package)).await }
#[tauri::command]
async fn execute_operation(app: tauri::AppHandle, token: String) -> Result<backend::OperationResult,String> {
    let info = backend::operation_info(&token);
    let result = blocking(move || backend::execute_operation(token)).await;
    if let Some(info) = info {
        let success = matches!(&result, Ok(done) if done.success);
        finish_operation(&app, &info, success).await;
    }
    result
}

/// What to do when an operation ends, given the state of the window.
#[derive(Debug, PartialEq, Eq)]
struct AfterOperation { notify: bool, quit: bool, show_window: bool }

/// `hidden`: the window is closed to the background (or missing). The user is always told that an
/// operation ended, even with the window open: a store that installs silently feels broken. If the
/// window had been closed meanwhile, quit on success, or bring it back on failure so the details
/// can be read.
fn after_operation(hidden: bool, success: bool) -> AfterOperation {
    AfterOperation { notify: true, quit: hidden && success, show_window: hidden && !success }
}

async fn finish_operation(app: &tauri::AppHandle, info: &backend::OperationInfo, success: bool) {
    let window = app.get_webview_window("main");
    let hidden = window.as_ref().map(|w| !w.is_visible().unwrap_or(true)).unwrap_or(true);
    let next = after_operation(hidden, success);
    if next.notify {
        let (summary, mut body) = notify::finished_message(&info.action, &info.label, success);
        if success && info.action == "check" {
            // Say what the check found, not just that it ended.
            let found = blocking(|| Ok(backend::list_updates("all".into()).ok().map(|updates| updates.len()))).await.ok().flatten();
            body = notify::updates_found_message(found);
        }
        if success && info.action == "install" && info.source == "flatpak" && backend::flatpak_menu_hidden() {
            body = format!("{body} {}", notify::menu_hint());
        }
        // Quitting right after notifying: GNOME Shell would delete a notification linked to the app the
        // moment it leaves the bus, so send one that stays until the user dismisses it.
        let delivery = if next.quit { notify::Delivery::Detached } else { notify::Delivery::Attached };
        let _ = blocking(move || Ok(notify::send(&summary, &body, delivery))).await;
    }
    if next.quit { app.exit(0); }
    else if next.show_window {
        if let Some(window) = window { let _ = window.show(); let _ = window.unminimize(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_happens_when_an_operation_ends() {
        // Window open: always notify, nothing else to do (the window shows the result).
        assert_eq!(after_operation(false, true), AfterOperation { notify: true, quit: false, show_window: false });
        assert_eq!(after_operation(false, false), AfterOperation { notify: true, quit: false, show_window: false });
        // Window closed to the background: notify, then quit when all went well...
        assert_eq!(after_operation(true, true), AfterOperation { notify: true, quit: true, show_window: false });
        // ...or come back with the details when it failed.
        assert_eq!(after_operation(true, false), AfterOperation { notify: true, quit: false, show_window: true });
    }
}

pub fn run() {
    // `cargo build --release` does not enable Tauri's `custom-protocol` feature, so the binary
    // would look for the dev server (127.0.0.1:1420) and show "Connection refused". Stop with
    // a clear message instead. `tauri dev` uses a debug build and is unaffected.
    if !cfg!(debug_assertions) && tauri::is_dev() {
        eprintln!("Nexa Store: this release executable was built with 'cargo build' and looks for the development server.\nRebuild with 'npm run tauri -- build --no-bundle' or 'npm run package'.");
        std::process::exit(1);
    }
    tauri::Builder::default()
        // Must be registered first. Without it, a second launch (for example a second click
        // on the menu icon) re-activates the running GTK application and makes it panic with
        // "a webview with label `main` already exists". The mutation gate is per-process, so a
        // single instance also keeps two windows from preparing package operations at once.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            // Match the window background to the system theme so there is no dark flash in a light
            // desktop (or the other way round) before the page has painted.
            if let Some(window) = app.get_webview_window("main") {
                let color = if matches!(window.theme(), Ok(tauri::Theme::Light)) { Color(243, 244, 248, 255) } else { Color(18, 21, 27, 255) };
                let _ = window.set_background_color(Some(color));
            }
            // Read the app catalogues in the background so Explore opens instantly.
            std::thread::spawn(store::warm_up);
            Ok(())
        })
        // Closing the window while a package operation runs must not stop it: hide the window, say so,
        // and let the operation finish (see `finish_operation`).
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if backend::operation_running() {
                    api.prevent_close();
                    let _ = window.hide();
                    let info = backend::running_operation();
                    std::thread::spawn(move || {
                        let (summary, body) = match info {
                            Some(info) => notify::background_message(&info.action, &info.label),
                            None => (tr!("Nexa Store keeps going in the background"), tr!("An operation is still running. You'll get a notification as soon as it ends.")),
                        };
                        notify::send(&summary, &body, notify::Delivery::Attached);
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![app_language,system_info,search_packages,list_installed,list_updates,package_details,prepare_operation,execute_operation,explore_home,browse_category,open_app])
        .run(tauri::generate_context!())
        .expect("cannot start Nexa Store");
}
