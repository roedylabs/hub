//! Per-user settings: language and appearance as one JSON document in
//! `user_settings` with a version.
//!
//! - [`Settings`] knows every field with the values the web app offers.
//!   Without a row and for missing fields the default applies.
//! - [`Settings::apply`] applies a change from `PUT settings` and checks each
//!   field on its own.
//! - [`load`] and [`save`] read and store with a version. Stored data this
//!   program version does not accept (e.g. after a downgrade) falls back to
//!   the default field by field.
//!
//! Taken over from Roedy Mail (`crates/server/src/settings.rs`) and trimmed
//! to language, color scheme and accent; the field names and values are the
//! same as in Mail's web app.

use rusqlite::Connection;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use crate::db::repo::{user_settings, users};

/// A setting with a fixed choice, serialized as a stable identifier.
macro_rules! choice {
    ($(#[$meta:meta])* $name:ident {
        $($(#[$variant_meta:meta])* $variant:ident = $value:literal),+ $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name {
            $($(#[$variant_meta])* #[serde(rename = $value)] $variant),+
        }
    };
}

choice!(
    /// Color scheme; without a choice the web app follows the system.
    ColorScheme { Light = "light", Dark = "dark", System = "system" }
);
choice!(
    /// Language of the web app and the server's texts; English unless chosen
    /// otherwise. `System` follows the browser: the web app asks it, server
    /// texts use the language of the last sign-in ([`Language::or_sign_in`]).
    #[derive(Default)]
    Language {
        German = "de",
        #[default]
        English = "en",
        System = "system",
    }
);

impl ColorScheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::System => "system",
        }
    }
}

impl Language {
    /// Language code, as serialized.
    pub fn code(self) -> &'static str {
        match self {
            Self::German => "de",
            Self::English => "en",
            Self::System => "system",
        }
    }

    /// The language texts are written in: `System` becomes `sign_in`, the
    /// language of the last sign-in, without one English.
    pub fn or_sign_in(self, sign_in: Option<Self>) -> Self {
        match self {
            Self::System => match sign_in {
                Some(Self::German) => Self::German,
                _ => Self::English,
            },
            fixed => fixed,
        }
    }

    /// The language a browser prefers, from `Accept-Language`: German for
    /// `de` and its regions, English for every other language; `None`
    /// without a language (empty, only `*` or `q=0`).
    pub fn from_accept_language(header: &str) -> Option<Self> {
        let mut best: Option<(&str, f32)> = None;
        for entry in header.split(',') {
            let mut parts = entry.split(';').map(str::trim);
            let tag = parts.next().unwrap_or_default();
            let weight = parts
                .find_map(|param| {
                    param
                        .strip_prefix("q=")
                        .or_else(|| param.strip_prefix("Q="))
                })
                .map_or(Some(1.0), |q| {
                    q.parse::<f32>().ok().filter(|q| (0.0..=1.0).contains(q))
                });
            let Some(weight) = weight else { continue };
            if tag.is_empty() || tag == "*" || weight <= 0.0 {
                continue;
            }
            // The first of equal weights wins.
            if best.is_none_or(|(_, top)| weight > top) {
                best = Some((tag, weight));
            }
        }
        let (tag, _) = best?;
        let primary = tag.split(['-', '_']).next().unwrap_or_default();
        Some(if primary.eq_ignore_ascii_case("de") {
            Self::German
        } else {
            Self::English
        })
    }
}

/// Accent color; without a choice the brand color applies. One of the five
/// named accents or a custom color, stored as `#rrggbb` in lowercase
/// (accepted: `#rgb`, `rgb`, `#rrggbb`, `rrggbb`, any case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Accent {
    Apricot,
    Sage,
    Lavender,
    Sky,
    Rose,
    /// Custom color as red, green and blue.
    Custom([u8; 3]),
}

impl Accent {
    /// Identifier as stored: the name or `#rrggbb`.
    pub fn id(self) -> String {
        let name = match self {
            Self::Apricot => "apricot",
            Self::Sage => "sage",
            Self::Lavender => "lavender",
            Self::Sky => "sky",
            Self::Rose => "rose",
            Self::Custom([r, g, b]) => return format!("#{r:02x}{g:02x}{b:02x}"),
        };
        name.to_owned()
    }

    /// A custom color from hex digits: `#rgb`, `rgb`, `#rrggbb` or `rrggbb`,
    /// any case.
    pub fn hex(value: &str) -> Option<Self> {
        let digits = value.strip_prefix('#').unwrap_or(value);
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |range: std::ops::Range<usize>| u8::from_str_radix(&digits[range], 16).ok();
        let rgb = match digits.len() {
            3 => [byte(0..1)? * 17, byte(1..2)? * 17, byte(2..3)? * 17],
            6 => [byte(0..2)?, byte(2..4)?, byte(4..6)?],
            _ => return None,
        };
        Some(Self::Custom(rgb))
    }
}

impl TryFrom<String> for Accent {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let named = match value.as_str() {
            "apricot" => Self::Apricot,
            "sage" => Self::Sage,
            "lavender" => Self::Lavender,
            "sky" => Self::Sky,
            "rose" => Self::Rose,
            other => return Self::hex(other).ok_or("accent name or hex color"),
        };
        Ok(named)
    }
}

impl From<Accent> for String {
    fn from(accent: Accent) -> Self {
        accent.id()
    }
}

/// The settings document. Missing fields take the default, unknown ones make
/// reading fail (then [`load`] takes over field by field).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub lang: Language,
    /// `None`: no choice yet (the web app follows the system).
    pub mode: Option<ColorScheme>,
    /// `None`: the brand color.
    pub accent: Option<Accent>,
}

/// Fields of a change that could not be applied: name and code (`unknown`,
/// `invalid`).
#[derive(Debug, PartialEq, Eq)]
pub struct Rejected(pub Vec<(String, &'static str)>);

impl Settings {
    /// The settings with `changes` applied. Every field is checked; a single
    /// bad one rejects the whole change. `null` clears `mode` and `accent`.
    pub fn apply(&self, changes: &Map<String, Value>) -> Result<Self, Rejected> {
        let mut rejected = Vec::new();
        let next = self.merge(changes, |field, code| {
            rejected.push((field.to_owned(), code));
        });
        if rejected.is_empty() {
            Ok(next)
        } else {
            Err(Rejected(rejected))
        }
    }

    /// Takes over every field of `changes` that fits; reports the others.
    fn merge(
        &self,
        changes: &Map<String, Value>,
        mut reject: impl FnMut(&str, &'static str),
    ) -> Self {
        fn parse<T: DeserializeOwned>(value: &Value) -> Option<T> {
            T::deserialize(value).ok()
        }
        let mut next = self.clone();
        for (field, value) in changes {
            let ok = match field.as_str() {
                "lang" => parse(value).map(|lang| next.lang = lang).is_some(),
                "mode" => parse(value).map(|mode| next.mode = mode).is_some(),
                "accent" => parse(value).map(|accent| next.accent = accent).is_some(),
                _ => {
                    reject(field, "unknown");
                    continue;
                }
            };
            if !ok {
                reject(field, "invalid");
            }
        }
        next
    }

    /// Color scheme for the apps (`theme` in `GET session`).
    pub fn theme(&self) -> ColorScheme {
        self.mode.unwrap_or(ColorScheme::System)
    }
}

/// Settings with the version they are based on (0 = never stored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub version: i64,
    pub settings: Settings,
    /// The stored document contains something this version skips; the next
    /// `PUT` rewrites it, even if nothing changes.
    pub rewrite: bool,
    /// Language of the server's texts and of the apps (`locale`):
    /// `settings.lang`, for `system` the language of the last sign-in.
    pub lang: Language,
}

/// Reads the settings, filled up with the defaults. What this version does
/// not accept falls back to the default field by field (logged); a document
/// that is not an object at all gives the defaults. Only a database error
/// fails.
pub fn load(conn: &Connection, user_id: &str) -> anyhow::Result<Stored> {
    let (version, settings, rewrite) = match user_settings::get(conn, user_id)? {
        None => (0, Settings::default(), false),
        Some(row) => {
            let doc: Value = serde_json::from_str(&row.json).unwrap_or(Value::Null);
            match &doc {
                Value::Object(fields) => match Settings::deserialize(&doc) {
                    Ok(settings) => (row.version, settings, false),
                    Err(_) => {
                        let settings = Settings::default().merge(fields, |field, code| {
                            tracing::warn!(user_id, field, code, "stored setting skipped");
                        });
                        (row.version, settings, true)
                    }
                },
                _ => {
                    tracing::warn!(
                        user_id,
                        "stored settings are not an object, using the defaults"
                    );
                    (row.version, Settings::default(), true)
                }
            }
        }
    };
    let sign_in = match settings.lang {
        Language::System => users::sign_in_lang(conn, user_id)?,
        _ => None,
    };
    let lang = settings.lang.or_sign_in(sign_in);
    Ok(Stored {
        version,
        settings,
        rewrite,
        lang,
    })
}

/// Stores `settings` as the successor of `version` and returns the new
/// version; `None` if another one applies by now.
pub fn save(
    conn: &Connection,
    user_id: &str,
    version: i64,
    settings: &Settings,
) -> anyhow::Result<Option<i64>> {
    let json = serde_json::to_string(settings)?;
    Ok(user_settings::save(conn, user_id, &json, version)?)
}

/// A user's language for the server's texts ([`Stored::lang`]).
pub fn lang_of(conn: &Connection, user_id: &str) -> anyhow::Result<Language> {
    Ok(load(conn, user_id)?.lang)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::db::repo::users::{
        Role,
        tests::{add, conn},
    };

    fn changes(value: Value) -> Map<String, Value> {
        let Value::Object(map) = value else {
            panic!("not an object");
        };
        map
    }

    #[test]
    fn accept_language_picks_german_or_english() {
        for (header, lang) in [
            ("de-DE,de;q=0.9,en;q=0.8", Some(Language::German)),
            ("en-US,en;q=0.9,de;q=0.8", Some(Language::English)),
            ("fr-FR", Some(Language::English)),
            ("fr;q=0.5, DE_at;q=0.9", Some(Language::German)),
            ("de;q=0, en;q=0.1", Some(Language::English)),
            ("*", None),
            ("", None),
            ("de;q=0", None),
            ("de;q=2", None),
        ] {
            assert_eq!(Language::from_accept_language(header), lang, "{header}");
        }
        assert_eq!(
            Language::System.or_sign_in(Some(Language::German)),
            Language::German
        );
        assert_eq!(Language::System.or_sign_in(None), Language::English);
        assert_eq!(
            Language::German.or_sign_in(Some(Language::English)),
            Language::German
        );
    }

    #[test]
    fn accents_by_name_or_hex() {
        for (input, id) in [
            ("sage", "sage"),
            ("#ABC", "#aabbcc"),
            ("abc", "#aabbcc"),
            ("#12aB9f", "#12ab9f"),
        ] {
            assert_eq!(Accent::try_from(input.to_owned()).unwrap().id(), id);
        }
        for bad in ["teal", "#abcd", "#ggg", "", "#"] {
            assert!(Accent::try_from(bad.to_owned()).is_err(), "{bad}");
        }
    }

    #[test]
    fn apply_checks_every_field() {
        let base = Settings::default();
        let next = base
            .apply(&changes(
                json!({ "lang": "de", "mode": "dark", "accent": "#F80" }),
            ))
            .unwrap();
        assert_eq!(next.lang, Language::German);
        assert_eq!(next.mode, Some(ColorScheme::Dark));
        assert_eq!(next.accent, Some(Accent::Custom([0xff, 0x88, 0x00])));
        assert_eq!(next.theme(), ColorScheme::Dark);
        let cleared = next
            .apply(&changes(json!({ "mode": null, "accent": null })))
            .unwrap();
        assert_eq!((cleared.mode, cleared.accent), (None, None));
        assert_eq!(cleared.theme(), ColorScheme::System);

        let rejected = base
            .apply(&changes(
                json!({ "lang": "fr", "mode": null, "density": "compact", "accent": 3 }),
            ))
            .unwrap_err();
        let mut fields = rejected.0;
        fields.sort();
        assert_eq!(
            fields,
            [
                ("accent".to_owned(), "invalid"),
                ("density".to_owned(), "unknown"),
                ("lang".to_owned(), "invalid"),
            ]
        );
        assert!(base.apply(&changes(json!({ "lang": null }))).is_err());
    }

    #[test]
    fn loads_defaults_and_stored_versions() {
        let conn = conn();
        let id = add(&conn, "max", "max@example.com", Role::User);
        let stored = load(&conn, &id).unwrap();
        assert_eq!(stored.version, 0);
        assert_eq!(stored.settings, Settings::default());
        assert_eq!(stored.lang, Language::English);

        let settings = Settings {
            lang: Language::German,
            mode: Some(ColorScheme::Light),
            accent: None,
        };
        assert_eq!(save(&conn, &id, 0, &settings).unwrap(), Some(1));
        assert_eq!(save(&conn, &id, 0, &settings).unwrap(), None, "outdated");
        let stored = load(&conn, &id).unwrap();
        assert_eq!((stored.version, stored.lang), (1, Language::German));
        assert_eq!(stored.settings, settings);
        assert!(!stored.rewrite);
        assert_eq!(lang_of(&conn, &id).unwrap(), Language::German);
    }

    #[test]
    fn system_language_follows_the_last_sign_in() {
        let conn = conn();
        let id = add(&conn, "max", "max@example.com", Role::User);
        let settings = Settings {
            lang: Language::System,
            ..Settings::default()
        };
        save(&conn, &id, 0, &settings).unwrap();
        assert_eq!(lang_of(&conn, &id).unwrap(), Language::English);
        users::set_sign_in_lang(&conn, &id, Language::German).unwrap();
        assert_eq!(lang_of(&conn, &id).unwrap(), Language::German);
    }

    #[test]
    fn unreadable_fields_fall_back_one_by_one() {
        let conn = conn();
        let id = add(&conn, "max", "max@example.com", Role::User);
        conn.execute(
            "INSERT INTO user_settings (user_id, json, version) VALUES (?1, ?2, 3)",
            (&id, r#"{"lang":"de","mode":"dim","future":1}"#),
        )
        .unwrap();
        let stored = load(&conn, &id).unwrap();
        assert_eq!(stored.version, 3);
        assert_eq!(stored.settings.lang, Language::German);
        assert_eq!(stored.settings.mode, None);
        assert!(stored.rewrite);

        conn.execute(
            "UPDATE user_settings SET json = '[1]' WHERE user_id = ?1",
            [&id],
        )
        .unwrap();
        let stored = load(&conn, &id).unwrap();
        assert_eq!(stored.settings, Settings::default());
        assert!(stored.rewrite);
    }
}
