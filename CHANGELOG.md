# Changelog

## 0.7.0 — 2026-10-19

Every Debian or Ubuntu based distribution, not only Ubuntu.

- **Debian, Ubuntu and their derivatives are supported.** The system check reads `ID` and `ID_LIKE` from `/etc/os-release` instead of requiring `ID=ubuntu`, so Debian, Linux Mint, Pop!_OS, Zorin OS, elementary OS, MX Linux, Kali, Raspberry Pi OS, AnduinOS and the others work; a derivative that does not declare its family is accepted when dpkg and APT manage it. Before, all of them showed "Unsupported system: Ubuntu is required." and every operation was refused.
- **APT catalog on older releases.** The AppStream metadata is also read from `/var/lib/app-info`, where Debian 12, Ubuntu 22.04 and Linux Mint 21 keep it (AppStream 1.0 moved it to `/var/lib/swcatalog`). Each real folder is read once when one links to the other.
- **System Flatpak remotes.** Many derivatives configure Flathub for the whole system and no user remote. New installs now go to the scope of the remote that offers the app (system-wide from a system remote, per user from a user one) instead of requiring a user remote, and "Check for updates" refreshes the catalog of system remotes too. System-wide changes no longer pass `--noninteractive`, so Flatpak can ask polkit for a password when its policy needs one. The warning appears only when no remote is configured at all.
- **No Snap warning on systems without Snap.** Linux Mint, AnduinOS, Debian and others leave Snap out on purpose: the source is just off. The warning stays for a `snap` command whose snapd does not answer.
- Removal protection covers the desktop and system meta-packages of the derivatives too (Debian tasks, Linux Mint, Pop!_OS, Zorin OS, elementary OS, Kali, Raspberry Pi OS, the Ubuntu flavours, display managers and desktop shells).
- The Debian package depends on `pkexec | policykit-1` and recommends `libfuse2t64 | libfuse2`, `flatpak`, `packagekit` and `libnotify-bin`, so it installs on older releases as well.
- `scripts/setup.sh` (formerly `setup-ubuntu.sh`, which still works) recognises every Debian or Ubuntu based system, picks the package names of that release and uses the distribution's Rust and Node.js only when they are recent enough.
- Messages that named Ubuntu now speak of "the system"; the README has a "Supported distributions" section.

## 0.6.1 — 2026-10-16

One button to check for updates.

- **The Updates page has a single "Check for updates" button.** It refreshes the APT package lists, the Flatpak catalog and the Snap list, one after the other, and then shows what can be updated. Before, the page only read the lists already on the computer and the refresh needed three separate buttons (APT, Flatpak, Snap): updates from third-party repositories, such as Microsoft Edge or ChatGPT, did not appear until the APT lists were refreshed by hand.
- **No password for the check.** The APT lists are refreshed through PackageKit, the service the Ubuntu Software Updater uses, which Ubuntu lets a person at the computer do without a password (polkit action `org.freedesktop.packagekit.system-sources-refresh`). Nothing is added to the system: no polkit rule, no helper. If PackageKit is missing or refuses, the standard `pkexec apt-get update` is used instead and the result says so. Installing, updating and removing APT packages still asks for the password.
- A source that is missing or fails does not stop the others, and the result lists each step. The notification at the end says how many updates were found; AppImage releases are looked up afresh.
- `scripts/test-gui.py` no longer runs any operation unless `NEXA_RUN_CHECK=1` (which runs the real check); `scripts/test-background.py` starts its Snap check through the backend commands.

## 0.6.0 — 2026-10-15

AppImage, and an interface in English and Italian.

- **New source: AppImage.** A catalog of about 2,900 apps from [AppImageHub](https://appimage.github.io) (the projects hosted on GitHub), searchable and browsable like the other sources, with the same app appearing once when it also exists in APT, Flatpak or Snap. Install, update, remove and open, per user, in `~/.local/share/nexa-store/appimages/`, with a launcher in the application menu and no administrator password.
- **Verified downloads.** The AppImage comes from the project's latest stable GitHub release, over https, from GitHub hosts only (checked on every redirect). Its size and the SHA-256 GitHub publishes for it must match, and it must have the AppImage file header, before it becomes executable. The confirmation lists the file, the size, the checksum and says plainly that an AppImage is not sandboxed.
- **The interface follows the system language**: Italian when the system language is Italian, English for every other language, resolved like gettext (`LC_ALL`, `LC_MESSAGES`, `LANG`, `LANGUAGE`). `NEXA_LANG=it|en` overrides it. The backend messages (confirmations, errors, notifications) follow it too, and so do app names and descriptions from the catalogs.
- Internal category ids are now English (`games`, `graphics`, …), and the "Graphics & photo" category is called "Graphics" in English.
- The `.desktop` file has an Italian generic name, comment and keywords; the package recommends `libfuse2t64` so AppImages start directly.
- New checks: `npm run test:i18n`, `scripts/test-language.py` (the app against ten locale settings), `scripts/test-appimage.py` (a real install and removal in an isolated folder); `scripts/test-gui.py` and `scripts/test-background.py` run in English or Italian.
- The README documents the language handling and the AppImage source.

## 0.5.0 — 2026-10-15

Background installs that you can actually see.

- **The final notification no longer disappears.** GNOME Shell deletes an app's notifications as soon as the app leaves the D-Bus. The store sent the "installed" notification and quit right away, so the notification was deleted about a second later, before it could be seen. A notification sent just before quitting is now not linked to the app and stays until dismissed. The background test proves it on the D-Bus.
- **Installed Flatpak apps now show up.** With Flatpak 1.18 the ref printed by `flatpak list` has no `app/` prefix, and the parser silently dropped every installed Flatpak app. They were missing from *Installed*, cards showed *Install* for apps that were already there, and they could not be removed or updated from the store.
- **Open button** on installed apps (cards, lists, details): starts the Flatpak, Snap or APT app without going through the application menu. A command-line-only snap gets a clear message instead.
- **Application-menu notice**: if the session started before the first Flatpak was installed, Flatpak apps do not appear in the menu, the dock or the search until the next login. The store now says so and repeats it in the end-of-install notification.
- `scripts/test-background.py` checks that the final notification is not deleted after the app exits.

## 0.4.1 — 2026-10-15

- **End-of-operation notification is always sent.** Before, it was skipped when the window was open and focused, so installing with the store open produced no notification at all. It now arrives for success and failure alike; if D-Bus does not answer, `notify-send` is tried.
- **Dock icon fixed.** The window identifies itself as `nexa-store`, but the `.desktop` file declared `StartupWMClass=it.nexa.store`, so the shell did not match them and showed a generic icon. Fixed in the package template, in `install-launcher.sh` and in the documentation.
- **New macOS-style icon**: squircle shape, blue gradient with a highlight and an inner rim; same symbol (a shopping bag with an "N").
- `scripts/test-background.py` also checks the notification with the window open.

## 0.4.0 — 2026-10-15

Operations that keep going with the window closed, with notifications.

- **You can close Nexa Store while it installs, removes or updates.** The window hides, the operation continues, one notification says it goes on in the background and another says when it is done; then the app quits by itself. If the operation fails, the window comes back with the details.
- **Execution independent of the app.** Operations that change the system write their output to a private temporary file and run in their own process group. Before, the output went through pipes owned by the app, so stopping the app could kill APT half-way (SIGPIPE).
- The confirmation dialog and the Operations page remind you that you can close the window.
- New `scripts/test-background.py`; new dependency `notify-rust` (it uses the D-Bus stack already included).

## 0.3.0 — 2026-10-15

New look and a new Explore page.

- **The interface was redesigned like an ordinary app store**: always-visible search bar, cards with logos, light and dark themes following the system, the Ubuntu font. Slogans and promotional text were removed.
- **Explore**: a rotating highlighted app, ten **categories** (Games, Internet, Audio & video, Graphics & photo, Office, Development, Education, Science, Utilities, System) and **new proposals on every launch**, with a "show more apps" button. Three different category sections on every visit.
- **Category page** with every app from the three sources, A–Z or random order, a source filter and paging.
- **One card per app** even if it is in APT, Flatpak and Snap; the source is chosen in the details. Italian names and summaries when available; APT shows the app's name instead of the package name.
- **Search**: GIMP now ranks above its plugins, the App/Package counts are per app, technical packages stay apart.
- **Details**: the real AppStream description for Flatpak apps (it used to show the output of `flatpak info`), text that reflows to the window width, no repeated title.
- **New store icon**, used in the app, the package and the menu.
- Updates: a **Snap** button to list updates (`snap refresh --list`, read-only).
- Fixed a WebKitGTK painting glitch that left a black square in the window corner after scrolling (caused by a CSS `filter` effect).
- `scripts/test-gui.py` updated for the new interface.

## 0.2.0 — 2026-10-15

- Added the Snap source: search, details, installed apps and per-package updates, with install, remove and update behind a preview and a confirmation, and a state re-check before running.
- Snap operations are limited to strict apps from the stable channel; classic, devmode and system components are excluded. No global Snap refresh.
- Added the **Apps**, **Technical packages** and **All** categories: by default only everyday apps are listed, while libraries, data, documentation, runtimes and system components stay under technical packages, and every result shows its type.
- Classification is based on AppStream metadata (APT), `app/…` refs (Flatpak) and the type declared by snapd; a package is not an "app" just because it has an icon.
- Every app shows its **logo**: raster icons read locally from AppStream, the icon theme and installed snap folders, or the Snap Store https icon. A neutral icon appears when a logo does not exist or fails to load. SVGs are never used.

## 0.1.0 — 2026-10-15

- First desktop version for Ubuntu with Tauri 2, React, TypeScript and Rust.
- APT and Flatpak search and management with preview, confirmation and a session log.
- APT simulation and safeguards, single-use expiring plans and explicit Flatpak scopes.
- Added the `.deb` package, the Ubuntu setup script and the per-user launcher.
