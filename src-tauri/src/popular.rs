//! Well-known desktop apps. The local catalogs carry no download counts, so the home page
//! gives these a larger share of its proposals; everything else is still sampled at random.
//!
//! Entries are display names. They are compared after the same normalisation used to match one
//! app across sources (lowercase letters and digits only). A trailing `*` matches any name that
//! starts with the entry, e.g. `LibreOffice*` covers Writer, Calc and Impress.
use crate::categories::name_key;
use std::sync::OnceLock;

const POPULAR: &[&str] = &[
    // Internet
    "Firefox", "Chromium", "Google Chrome", "Brave", "Vivaldi", "Opera", "Thunderbird", "Telegram Desktop", "Telegram",
    "Signal", "Signal Desktop", "Discord", "Slack", "Zoom", "Element", "Transmission", "qBittorrent", "Deluge",
    "FileZilla", "Evolution", "Geary", "Pidgin", "Skype", "Fragments", "Tuba", "Mastodon", "Wireshark", "Remmina",
    "Syncthing", "Nextcloud*", "Dropbox", "Mullvad VPN", "ProtonVPN", "Proton Mail", "Bitwarden", "Tor Browser", "LibreWolf",
    // Audio and video
    "VLC", "OBS Studio", "Kdenlive", "Shotcut", "Audacity", "Spotify", "Rhythmbox", "Lollypop", "Celluloid", "MPV",
    "HandBrake", "OpenShot*", "Ardour", "Mixxx", "LMMS", "MuseScore*", "Kodi", "Clapper", "Amberol", "Strawberry",
    "Pitivi", "Cheese", "Easy Effects", "Jellyfin*", "Tenacity", "Shortwave", "Music Player*", "Sayonara", "Parole",
    "SMPlayer", "Haruna", "Tauon Music Box", "Cider", "Podcasts", "Foliate",
    // Graphics and photo
    "GIMP", "GNU Image Manipulation Program", "Inkscape", "Krita", "Blender", "Darktable", "RawTherapee", "digiKam", "Pinta",
    "Shotwell", "Scribus", "FreeCAD", "OpenSCAD", "Gwenview", "Loupe", "Drawing", "Flameshot", "KolourPaint", "MyPaint",
    "Upscayl", "UltiMaker Cura", "Cura", "PrusaSlicer", "LibreCAD", "Tux Paint", "Hugin", "Fotoxx", "Okular", "Evince",
    "Document Viewer", "Image Viewer", "Eye of GNOME", "Photoflare", "Glimpse", "Rnote", "Xournal++", "Gaphor", "draw.io",
    // Office
    "LibreOffice*", "ONLYOFFICE*", "WPS Office", "Calibre", "Zotero", "Joplin", "Obsidian", "Logseq", "Standard Notes",
    "GnuCash", "KMyMoney", "Planify", "Endeavour", "Simple Scan", "Zettlr", "MarkText", "KeePassXC", "Calendar",
    "Contacts", "Notes", "Apostrophe", "Ghostwriter", "FreeOffice*", "Evolution", "Collabora*", "Paperwork", "Pdf Arranger", "PDFsam*",
    // Development
    "Visual Studio Code", "VSCodium", "Sublime Text", "Android Studio", "IntelliJ IDEA*", "PyCharm*", "WebStorm", "Eclipse*",
    "Geany", "Kate", "Neovim", "GitKraken", "GitHub Desktop", "Postman", "Insomnia", "DBeaver*", "Meld", "Godot*",
    "Arduino IDE", "Qt Creator", "Builder", "GNOME Builder", "Text Editor", "Gedit", "Lite XL", "Zed", "Helix", "Bruno",
    "Beekeeper Studio", "Sqlitebrowser", "DB Browser for SQLite", "CLion", "Rider", "GoLand", "PhpStorm", "RubyMine",
    // Games
    "Steam", "Lutris", "Heroic Games Launcher", "RetroArch", "PCSX2", "Dolphin Emulator", "DuckStation", "PPSSPP", "RPCS3",
    "yuzu", "Ryujinx", "SuperTuxKart", "0 A.D.", "Minecraft*", "Luanti", "Minetest", "Battle for Wesnoth", "OpenTTD",
    "Xonotic", "Mindustry", "Bottles", "ProtonUp-Qt", "Mines", "Sudoku", "Chess", "Aisleriot Solitaire", "Quadrapassel",
    "Hedgewars", "Warzone 2100", "FreeCiv*", "Veloren", "Cataclysm*", "Red Eclipse", "Pingus", "Frozen Bubble", "Extreme Tux Racer",
    "Neverball", "Endless Sky", "Shattered Pixel Dungeon", "Albion Online", "Prism Launcher", "PolyMC", "Itch", "Gamehub",
    "SuperTux", "Supertux", "Armagetron*", "Teeworlds", "Freeciv", "OpenRA*", "Widelands", "Flare", "Simon Tatham's Portable Puzzle Collection",
    // Education and science
    "GCompris", "Stellarium", "Anki", "KStars", "Kiwix", "GeoGebra", "Scratch*", "Kalzium", "Celestia*", "Marble",
    "Klavaro", "Gramps", "GNU Octave", "Octave", "SageMath", "Spyder", "JupyterLab", "Gnuplot", "Avogadro*", "KiCad",
    "QGIS", "Maxima", "wxMaxima", "Scilab", "Fritzing", "Tux Math", "Tux Typing", "Kturtle", "Kig", "Cantor",
    // Utilities and system
    "Timeshift", "BleachBit", "GParted", "Stacer", "balenaEtcher", "Etcher", "Flatseal", "Mission Center", "Resources",
    "Disks", "GNOME Disks", "Files", "Nautilus", "Terminal", "Ptyxis", "Alacritty", "Kitty", "Tilix", "Ghostty", "Warp",
    "Extension Manager", "GNOME Tweaks", "Tweaks", "Solaar", "Boxes", "VirtualBox", "Virtual Machine Manager", "Disk Usage Analyzer",
    "Baobab", "System Monitor", "Btop*", "Htop", "Fastfetch", "Neofetch", "Peek", "Kooha", "Impression", "Ventoy*", "Cryptomator",
    "Yubico Authenticator", "Clipboard Indicator", "Gradience", "Pika Backup", "Déjà Dup", "Vorta", "Czkawka", "Fclones", "Gaphor",
];

struct Matcher { exact: Vec<String>, prefixes: Vec<String> }

fn matcher() -> &'static Matcher {
    static MATCHER: OnceLock<Matcher> = OnceLock::new();
    MATCHER.get_or_init(|| {
        let (mut exact, mut prefixes) = (Vec::new(), Vec::new());
        for entry in POPULAR {
            match entry.strip_suffix('*') { Some(prefix) => prefixes.push(name_key(prefix)), None => exact.push(name_key(entry)) }
        }
        exact.sort(); exact.dedup();
        Matcher { exact, prefixes }
    })
}

/// `key` is a normalised app name (see `categories::name_key`).
pub fn is_popular(key: &str) -> bool {
    if key.is_empty() { return false; }
    let matcher = matcher();
    matcher.exact.binary_search(&key.to_string()).is_ok() || matcher.prefixes.iter().any(|prefix| key.starts_with(prefix.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_prefix_entries_match_normalised_names() {
        assert!(is_popular(&name_key("GIMP")));
        assert!(is_popular(&name_key("Visual Studio Code")));
        assert!(is_popular(&name_key("LibreOffice Writer")), "prefisso LibreOffice*");
        assert!(!is_popular(&name_key("Gimpshop clone")), "entries without * must match in full");
        assert!(!is_popular(&name_key("kube-apiserver")));
        assert!(!is_popular(""));
    }
}
