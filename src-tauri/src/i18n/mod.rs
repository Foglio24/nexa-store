//! Language of the messages the backend produces (errors, confirmation texts, notifications).
//!
//! The interface is Italian when the system language is Italian and English for every other
//! language. Messages are written in English in the code; `tr!` looks the English text up in the
//! Italian table (`it.rs`) when the language is Italian and fills in the `{}` placeholders in order.
//! A message with no Italian entry is shown in English, never as an error. The tests below check
//! that every message in the code has an entry and that placeholders match.
use std::{collections::HashMap, fmt::Display, sync::OnceLock};

mod it;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang { En, It }

impl Lang {
    pub fn code(self) -> &'static str { match self { Lang::En => "en", Lang::It => "it" } }
}

/// `it_IT.UTF-8`, `it`, `en-GB`, `de_DE@euro` → `it`, `it`, `en`, `de`.
fn primary(tag: &str) -> String {
    tag.trim().chars().take_while(|c| !matches!(c, '_' | '-' | '.' | '@')).collect::<String>().to_ascii_lowercase()
}

/// Pick the language the way gettext does. `NEXA_LANG` (`it` or `en`) overrides everything, for
/// tests and for people who want a language other than the system's. Otherwise the first of
/// `LC_ALL`, `LC_MESSAGES` and `LANG` that is set gives the locale; `LANGUAGE`, a preference list
/// such as `de:it:en`, is consulted when the locale is not `C`/`POSIX`, and its first entry that is
/// Italian or English wins. Anything else is English.
pub fn detect(get: &dyn Fn(&str) -> Option<String>) -> Lang {
    let by_code = |code: &str| match code { "it" => Some(Lang::It), "en" => Some(Lang::En), _ => None };
    if let Some(forced) = get("NEXA_LANG").and_then(|value| by_code(&primary(&value))) { return forced; }
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|name| get(name)).find(|value| !value.trim().is_empty());
    let Some(locale) = locale else { return Lang::En };
    if matches!(locale.trim(), "C" | "POSIX") { return Lang::En; }
    if let Some(list) = get("LANGUAGE").filter(|list| !list.trim().is_empty()) {
        return list.split(':').find_map(|entry| by_code(&primary(entry))).unwrap_or(Lang::En);
    }
    by_code(&primary(&locale)).unwrap_or(Lang::En)
}

#[cfg(test)]
thread_local! { static FORCED: std::cell::Cell<Option<Lang>> = const { std::cell::Cell::new(None) }; }

/// The language of this run. Fixed at the first call; unit tests default to English.
pub fn current() -> Lang {
    #[cfg(test)]
    { return FORCED.with(|forced| forced.get()).unwrap_or(Lang::En); }
    #[cfg(not(test))]
    {
        static DETECTED: OnceLock<Lang> = OnceLock::new();
        *DETECTED.get_or_init(|| detect(&|name| std::env::var(name).ok()))
    }
}

pub fn italian() -> bool { current() == Lang::It }

/// Run `work` with the given language on this thread (tests only).
#[cfg(test)]
pub fn with_language<T>(language: Lang, work: impl FnOnce() -> T) -> T {
    let previous = FORCED.with(|forced| forced.replace(Some(language)));
    let result = work();
    FORCED.with(|forced| forced.set(previous));
    result
}

fn italian_table() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| it::MESSAGES.iter().copied().collect())
}

/// Replace each `{}` in `template` with the next argument. Surplus placeholders stay as written.
fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut result = String::with_capacity(template.len() + 16);
    let mut values = args.iter();
    let mut rest = template;
    while let Some(position) = rest.find("{}") {
        result.push_str(&rest[..position]);
        match values.next() { Some(value) => result.push_str(&value.to_string()), None => result.push_str("{}") }
        rest = &rest[position + 2..];
    }
    result.push_str(rest);
    result
}

pub fn translate(message: &'static str, args: &[&dyn Display]) -> String {
    let template = if italian() { italian_table().get(message).copied().unwrap_or(message) } else { message };
    fill(template, args)
}

/// `tr!("English text")` or `tr!("Copied {} of {}", done, total)`: the message in the interface
/// language. The text must be a string literal so the tests can find it.
macro_rules! tr {
    ($message:literal) => { $crate::i18n::translate($message, &[]) };
    ($message:literal, $($argument:expr),+ $(,)?) => { $crate::i18n::translate($message, &[$(&$argument as &dyn std::fmt::Display),+]) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn italian_system_gives_italian_and_everything_else_english() {
        assert_eq!(detect(&environment(&[("LANG", "it_IT.UTF-8")])), Lang::It);
        assert_eq!(detect(&environment(&[("LANG", "it_CH.UTF-8")])), Lang::It);
        assert_eq!(detect(&environment(&[("LANG", "it")])), Lang::It);
        for other in ["en_US.UTF-8", "en_GB.UTF-8", "de_DE.UTF-8", "fr_FR", "es_ES.UTF-8", "pt_BR.UTF-8", "ja_JP.UTF-8", "ca_ES@valencia", "C", "POSIX", "C.UTF-8", "ita"] {
            assert_eq!(detect(&environment(&[("LANG", other)])), Lang::En, "{other}");
        }
        assert_eq!(detect(&environment(&[])), Lang::En, "no locale at all");
        assert_eq!(detect(&environment(&[("LANG", "")])), Lang::En);
    }

    #[test]
    fn locale_variables_follow_the_posix_order() {
        assert_eq!(detect(&environment(&[("LANG", "it_IT.UTF-8"), ("LC_MESSAGES", "en_US.UTF-8")])), Lang::En);
        assert_eq!(detect(&environment(&[("LANG", "en_US.UTF-8"), ("LC_MESSAGES", "it_IT.UTF-8")])), Lang::It);
        assert_eq!(detect(&environment(&[("LANG", "en_US.UTF-8"), ("LC_MESSAGES", "it_IT.UTF-8"), ("LC_ALL", "de_DE.UTF-8")])), Lang::En);
        assert_eq!(detect(&environment(&[("LANG", "en_US.UTF-8"), ("LC_ALL", "it_IT.UTF-8")])), Lang::It);
        assert_eq!(detect(&environment(&[("LC_ALL", ""), ("LANG", "it_IT.UTF-8")])), Lang::It, "an empty variable is ignored");
    }

    #[test]
    fn the_language_list_picks_the_first_language_that_has_a_translation() {
        assert_eq!(detect(&environment(&[("LANG", "de_DE.UTF-8"), ("LANGUAGE", "de:it:en")])), Lang::It, "Italian is the first available language");
        assert_eq!(detect(&environment(&[("LANG", "it_IT.UTF-8"), ("LANGUAGE", "en:it")])), Lang::En);
        assert_eq!(detect(&environment(&[("LANG", "it_IT.UTF-8"), ("LANGUAGE", "it")])), Lang::It);
        assert_eq!(detect(&environment(&[("LANG", "it_IT.UTF-8"), ("LANGUAGE", "de:fr")])), Lang::En, "no translation in the list: English, as gettext does");
        assert_eq!(detect(&environment(&[("LANG", "C"), ("LANGUAGE", "it")])), Lang::En, "LANGUAGE is ignored in the C locale");
        assert_eq!(detect(&environment(&[("LANG", "it_IT.UTF-8"), ("LANGUAGE", "")])), Lang::It);
    }

    #[test]
    fn nexa_lang_overrides_the_system() {
        assert_eq!(detect(&environment(&[("LANG", "it_IT.UTF-8"), ("NEXA_LANG", "en")])), Lang::En);
        assert_eq!(detect(&environment(&[("LANG", "de_DE.UTF-8"), ("NEXA_LANG", "it")])), Lang::It);
        assert_eq!(detect(&environment(&[("LANG", "it_IT.UTF-8"), ("NEXA_LANG", "xx")])), Lang::It, "an unknown override is ignored");
    }

    #[test]
    fn placeholders_are_filled_in_order() {
        assert_eq!(fill("{} of {}", &[&3, &"five"]), "3 of five");
        assert_eq!(fill("no placeholders", &[&1]), "no placeholders");
        assert_eq!(fill("{} and {}", &[&"a"]), "a and {}");
        assert_eq!(fill("{}{}", &[&"é", &"→"]), "é→");
    }

    #[test]
    fn a_message_is_translated_only_in_italian() {
        let english = "Another operation is in progress. Wait for it to finish.";
        assert_eq!(with_language(Lang::En, || translate(english, &[])), english);
        assert_eq!(with_language(Lang::It, || translate(english, &[])), "Un'altra operazione è in corso. Attendi il completamento.");
        assert_eq!(with_language(Lang::It, || translate("A message that has no entry {}", &[&1])), "A message that has no entry 1");
    }

    /// Every `tr!("…")` in the sources, with escapes resolved.
    fn messages_in_sources() -> Vec<(String, String)> {
        const SOURCES: [(&str, &str); 7] = [
            ("backend.rs", include_str!("../backend.rs")), ("store.rs", include_str!("../store.rs")), ("notify.rs", include_str!("../notify.rs")),
            ("lib.rs", include_str!("../lib.rs")), ("catalog.rs", include_str!("../catalog.rs")), ("appimage.rs", include_str!("../appimage.rs")),
            ("categories.rs", include_str!("../categories.rs")),
        ];
        let mut found = Vec::new();
        for (file, text) in SOURCES {
            let mut rest = text;
            while let Some(start) = rest.find("tr!(") {
                // Only the macro itself: `tr!(` must not be the tail of another identifier.
                let before = rest[..start].chars().next_back();
                rest = &rest[start + 4..];
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') { continue; }
                let Some(literal) = rest.trim_start().strip_prefix('"') else { continue };
                let (mut message, mut chars) = (String::new(), literal.chars());
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() { Some('n') => message.push('\n'), Some('t') => message.push('\t'), Some(other) => message.push(other), None => break },
                        other => message.push(other),
                    }
                }
                found.push((file.to_string(), message));
            }
        }
        found
    }

    #[test]
    fn every_message_in_the_code_has_an_italian_entry() {
        let table = italian_table();
        let missing: Vec<_> = messages_in_sources().into_iter().filter(|(_, message)| !table.contains_key(message.as_str())).collect();
        assert!(missing.is_empty(), "no Italian text for: {missing:#?}");
    }

    #[test]
    fn the_table_has_no_stale_or_malformed_entries() {
        let used: std::collections::HashSet<String> = messages_in_sources().into_iter().map(|(_, message)| message).collect();
        let mut seen = std::collections::HashSet::new();
        for (english, italian) in it::MESSAGES {
            assert!(seen.insert(*english), "duplicate entry: {english}");
            assert_eq!(english.matches("{}").count(), italian.matches("{}").count(), "placeholders differ: {english}");
            assert!(!italian.trim().is_empty() && italian != english, "not translated: {english}");
            assert!(used.contains(*english), "entry no longer used in the code: {english}");
        }
    }
}
