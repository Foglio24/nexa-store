//! Store categories and the mapping from each package source onto them.
//!
//! APT (DEP-11) and Flatpak (AppStream) both use the freedesktop menu categories;
//! the Snap Store uses its own "sections". Everything is mapped onto one small,
//! fixed list so the same category page works for every source.

/// Category ids in display order. The Italian labels and icons live in the frontend.
pub const CATEGORY_IDS: [&str; 10] = [
    "giochi", "internet", "audio-video", "grafica", "ufficio",
    "sviluppo", "istruzione", "scienza", "utilita", "sistema",
];

pub fn is_category(id: &str) -> bool { CATEGORY_IDS.contains(&id) }

/// Map one freedesktop category name (e.g. `ArcadeGame`, `Graphics`) onto a store category.
pub fn from_freedesktop(raw: &str) -> Option<&'static str> {
    if raw == "Game" || raw.ends_with("Game") { return Some("giochi"); }
    Some(match raw {
        "Network" | "WebBrowser" | "Email" | "InstantMessaging" | "Chat" | "IRCClient" | "FileTransfer"
        | "P2P" | "Feed" | "News" | "VideoConference" | "RemoteAccess" | "Telephony" => "internet",
        "AudioVideo" | "Audio" | "Video" | "Player" | "Music" | "Midi" | "Mixer" | "Sequencer" | "Tuner"
        | "Recorder" | "AudioVideoEditing" | "DiscBurning" | "Radio" | "TV" | "Jukebox" => "audio-video",
        "Graphics" | "2DGraphics" | "3DGraphics" | "RasterGraphics" | "VectorGraphics" | "Photography"
        | "Scanning" | "OCR" | "ImageProcessing" | "Publishing" => "grafica",
        "Office" | "WordProcessor" | "Spreadsheet" | "Presentation" | "Calendar" | "ContactManagement"
        | "Database" | "Dictionary" | "Chart" | "Finance" | "ProjectManagement" | "FlowChart" => "ufficio",
        "Development" | "IDE" | "Debugger" | "GUIDesigner" | "Profiling" | "RevisionControl"
        | "Translation" | "WebDevelopment" | "Building" => "sviluppo",
        "Education" | "Languages" | "Literature" | "Geography" | "History" | "ComputerScience" | "Art"
        | "ArtificialIntelligence" | "Construction" => "istruzione",
        "Science" | "Math" | "Astronomy" | "Biology" | "Chemistry" | "Geology" | "Physics" | "Electronics"
        | "Engineering" | "DataVisualization" | "NumericalAnalysis" | "MedicalSoftware" | "Robotics"
        | "Geoscience" | "ParallelComputing" => "scienza",
        "Utility" | "Accessibility" | "Archiving" | "Compression" | "Calculator" | "Clock" | "TextEditor"
        | "FileManager" | "TerminalEmulator" | "FileTools" | "TextTools" => "utilita",
        "System" | "Settings" | "Monitor" | "Security" | "PackageManager" | "HardwareSettings"
        | "DesktopSettings" | "Filesystem" | "Emulator" => "sistema",
        _ => return None,
    })
}

/// Store categories for a list of freedesktop names, without duplicates, in first-seen order.
pub fn from_freedesktop_list<S: AsRef<str>>(raw: &[S]) -> Vec<&'static str> {
    let mut result: Vec<&'static str> = Vec::new();
    for name in raw {
        if let Some(category) = from_freedesktop(name.as_ref()) {
            if !result.contains(&category) { result.push(category); }
        }
    }
    result
}

/// Snap Store section (as listed by snapd's `/v2/sections`) → store category.
pub fn from_snap_section(section: &str) -> Option<&'static str> {
    Some(match section {
        "games" => "giochi",
        "social" | "news-and-weather" => "internet",
        "music-and-audio" | "entertainment" => "audio-video",
        "photo-and-video" | "art-and-design" => "grafica",
        "productivity" | "finance" => "ufficio",
        "development" => "sviluppo",
        "education" | "books-and-reference" => "istruzione",
        "science" => "scienza",
        "utilities" | "personalisation" | "health-and-fitness" => "utilita",
        "security" | "server-and-cloud" | "devices-and-iot" => "sistema",
        _ => return None,
    })
}

/// Every Snap Store section that feeds the store categories (all but `featured`).
pub const SNAP_SECTIONS: [&str; 19] = [
    "art-and-design", "books-and-reference", "development", "devices-and-iot", "education",
    "entertainment", "finance", "games", "health-and-fitness", "music-and-audio",
    "news-and-weather", "personalisation", "photo-and-video", "productivity", "science",
    "security", "server-and-cloud", "social", "utilities",
];

/// Normalised key used to recognise the same app across sources.
pub fn name_key(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Normalised AppStream/desktop id (`Foo.Bar.desktop` → `foo.bar`).
pub fn id_key(id: &str) -> String {
    let id = id.trim().to_lowercase();
    id.strip_suffix(".desktop").map(str::to_string).unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freedesktop_categories_map_onto_store_categories() {
        assert_eq!(from_freedesktop("ArcadeGame"), Some("giochi"));
        assert_eq!(from_freedesktop("Game"), Some("giochi"));
        assert_eq!(from_freedesktop("WebBrowser"), Some("internet"));
        assert_eq!(from_freedesktop("Graphics"), Some("grafica"));
        assert_eq!(from_freedesktop("GTK"), None);
        assert_eq!(from_freedesktop("Qt"), None);
        let list = from_freedesktop_list(&["Graphics", "2DGraphics", "GTK", "Education", "RasterGraphics"]);
        assert_eq!(list, vec!["grafica", "istruzione"]);
    }

    #[test]
    fn every_mapped_target_is_a_known_category() {
        for raw in ["Game", "Network", "AudioVideo", "Graphics", "Office", "Development", "Education", "Science", "Utility", "System"] {
            assert!(is_category(from_freedesktop(raw).unwrap()), "{raw}");
        }
        for section in SNAP_SECTIONS { assert!(is_category(from_snap_section(section).unwrap()), "{section}"); }
        assert!(from_snap_section("featured").is_none());
        for id in CATEGORY_IDS { assert!(SNAP_SECTIONS.iter().any(|s| from_snap_section(s) == Some(id)), "no Snap section for {id}"); }
    }

    #[test]
    fn keys_identify_the_same_app_across_sources() {
        assert_eq!(name_key("GNU Image Manipulation Program"), name_key("gnu image-manipulation program"));
        assert_eq!(id_key("org.GIMP.GIMP"), id_key("org.gimp.GIMP.desktop"));
        assert_eq!(id_key(" firefox.desktop "), "firefox");
    }
}
