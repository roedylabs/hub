//! Server texts in the reader's language, such as device names and the
//! default name of an authenticator app. The texts live in
//! `locales/<lang>.json` as flat maps from key to text, embedded in the
//! binary and parsed once. Placeholders are written `{name}`.
//!
//! A key missing in a language falls back to English, a key missing in
//! English to the key itself; tests keep both files in step.
//!
//! Taken over from Roedy Mail (`crates/server/src/i18n.rs`) without plurals,
//! which follow with the mails.

use std::{
    collections::HashMap,
    fmt::Display,
    sync::{LazyLock, Mutex, PoisonError},
};

use crate::settings::Language;

type Texts = HashMap<String, String>;

fn parse(lang: &str, json: &str) -> Texts {
    serde_json::from_str(json).unwrap_or_else(|err| panic!("locales/{lang}.json: {err}"))
}

static ENGLISH: LazyLock<Texts> = LazyLock::new(|| parse("en", include_str!("../locales/en.json")));
static GERMAN: LazyLock<Texts> = LazyLock::new(|| parse("de", include_str!("../locales/de.json")));

fn texts(lang: Language) -> &'static Texts {
    match lang {
        // `System` is resolved before (`settings::Stored::lang`).
        Language::English | Language::System => &ENGLISH,
        Language::German => &GERMAN,
    }
}

/// The text of `key` in `lang`.
pub fn t(lang: Language, key: &str) -> &'static str {
    if let Some(text) = lookup(texts(lang), &ENGLISH, key) {
        return text;
    }
    // Unknown keys only reach release builds through a programming error;
    // showing the key beats panicking in a request.
    debug_assert!(false, "unknown text key {key}");
    unknown(key)
}

/// `key` in `texts`, otherwise in `fallback`.
fn lookup<'a>(texts: &'a Texts, fallback: &'a Texts, key: &str) -> Option<&'a str> {
    texts
        .get(key)
        .or_else(|| fallback.get(key))
        .map(String::as_str)
}

/// `key` itself as a text, kept once per key.
fn unknown(key: &str) -> &'static str {
    static KEYS: LazyLock<Mutex<HashMap<String, &'static str>>> = LazyLock::new(Mutex::default);
    let mut keys = KEYS.lock().unwrap_or_else(PoisonError::into_inner);
    keys.entry(key.to_owned())
        .or_insert_with(|| Box::leak(key.to_owned().into_boxed_str()))
}

/// The text of `key` in `lang` with its `{name}` placeholders replaced.
pub fn tf(lang: Language, key: &str, args: &[(&str, &dyn Display)]) -> String {
    fill(t(lang, key), args)
}

fn fill(text: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after.find('}').and_then(|close| {
            let name = &after[..close];
            let (_, value) = args.iter().find(|(key, _)| *key == name)?;
            Some((value, close))
        });
        match value {
            Some((value, close)) => {
                out.push_str(&value.to_string());
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// Placeholders of a text, sorted.
    fn placeholders(text: &str) -> Vec<&str> {
        let mut names: Vec<&str> = text
            .split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name))
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    #[test]
    fn both_languages_have_the_same_keys_and_placeholders() {
        let english: BTreeSet<_> = ENGLISH.keys().collect();
        let german: BTreeSet<_> = GERMAN.keys().collect();
        assert_eq!(english, german, "keys of en.json and de.json");
        for key in english {
            assert_eq!(
                placeholders(&ENGLISH[key]),
                placeholders(&GERMAN[key]),
                "placeholders of {key}"
            );
            assert!(!ENGLISH[key].is_empty() && !GERMAN[key].is_empty(), "{key}");
        }
    }

    #[test]
    fn fills_placeholders() {
        assert_eq!(fill("{a} and {b}", &[("a", &1), ("b", &"x")]), "1 and x");
        assert_eq!(fill("{unknown} {", &[]), "{unknown} {");
        assert_eq!(placeholders("{b} {a} {b}"), ["a", "b"]);
    }

    #[test]
    fn texts_come_from_the_language_and_fall_back_to_english() {
        assert_eq!(t(Language::English, "device.unknown"), "Unknown device");
        assert_eq!(
            t(Language::German, "device.unknown"),
            GERMAN["device.unknown"]
        );
        assert_eq!(t(Language::System, "device.unknown"), "Unknown device");
        let texts = |pairs: &[(&str, &str)]| -> Texts {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        let english = texts(&[("a", "en a"), ("b", "en b")]);
        let other = texts(&[("a", "other a")]);
        assert_eq!(lookup(&other, &english, "a"), Some("other a"));
        assert_eq!(lookup(&other, &english, "b"), Some("en b"));
        assert_eq!(lookup(&other, &english, "c"), None);
    }
}
