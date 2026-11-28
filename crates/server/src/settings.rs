//! Per-user settings: appearance, language and region and the shared
//! behaviour of every Roedy app, as one JSON document in `user_settings` with
//! a version (docs/plan.md § 9.2, milestone 9).
//!
//! - [`Settings`] knows every field with the values the web app offers.
//!   Without a row and for missing fields the default applies.
//! - [`Settings::apply`] applies a change from `PUT settings` and checks each
//!   field on its own. A new region sets the formats back to automatic.
//! - [`load`] and [`save`] read and store with a version. Stored data this
//!   program version does not accept (e.g. after a downgrade) falls back to
//!   the default field by field.
//! - The apps get the settings as `prefs`: all of them in the browser
//!   ([`Stored::browser_prefs`], `GET session`), what a backend needs for
//!   texts and mails in the internal API and `user.updated`
//!   ([`Stored::app_prefs`]). The hub's own mails use [`Stored::format`].
//!
//! Taken over from Roedy Mail (`crates/server/src/settings.rs`) with the
//! fields every app applies; Mail's own settings (list design, swipe
//! actions, external content, start page) stay in Mail, notifications come
//! with milestone 10. The field names follow the hub's API (snake case), the
//! formats are automatic (`null`: the region's, without one the language's).

use jiff::tz::TimeZone;
use roedy_hub_contract::pairing::UserPrefs;
use rusqlite::Connection;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use crate::{
    db::repo::{events, user_settings, users},
    mailer::format::{DEFAULT_TZ, Format},
};

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

choice!(
    /// Font size of the interface (`data-font-size`).
    #[derive(Default)]
    FontSize {
        Small = "small",
        #[default]
        Medium = "medium",
        Large = "large",
    }
);
choice!(
    /// Spacing of lists and rows (`data-density`).
    #[derive(Default)]
    Density {
        Compact = "compact",
        #[default]
        Normal = "normal",
        Comfortable = "comfortable",
    }
);
choice!(
    /// Country or region; it picks the automatic formats
    /// ([`Formats::of`]) and tells the apps where the user lives (holidays).
    Region {
        Germany = "DE",
        Austria = "AT",
        Switzerland = "CH",
        France = "FR",
        Italy = "IT",
        Netherlands = "NL",
        Spain = "ES",
        UnitedKingdom = "GB",
        UnitedStates = "US",
    }
);
choice!(
    /// Date in digits; mails spell `de` out with the month's name.
    DateFormat {
        /// "25.10.2026"
        De = "de",
        /// "25/10/2026"
        Dmy = "dmy",
        /// "2026-10-25"
        Iso = "iso",
        /// "10/25/2026"
        Us = "us",
    }
);
choice!(
    /// Clock.
    TimeFormat { H24 = "24h", H12 = "12h" }
);
choice!(
    /// First day of the week.
    WeekStart { Monday = "monday", Sunday = "sunday" }
);
choice!(
    /// Thousands and decimal separators.
    NumberFormat {
        Comma = "1.234,56",
        Point = "1,234.56",
        Space = "1\u{a0}234,56",
        Apostrophe = "1\u{2019}234.56",
    }
);

/// Serialized name of a [`choice!`] value.
fn name_of<T: Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => name,
        _ => String::new(),
    }
}

impl Region {
    pub fn code(self) -> String {
        name_of(self)
    }
}

/// The four formats, resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Formats {
    pub date: DateFormat,
    pub time: TimeFormat,
    pub week: WeekStart,
    pub number: NumberFormat,
}

impl Formats {
    /// What is usual in `region`; without one what fits the language
    /// (`lang` resolved, see [`Stored::lang`]).
    pub fn of(region: Option<Region>, lang: Language) -> Self {
        use {DateFormat as D, NumberFormat as N, Region as R, TimeFormat as T, WeekStart as W};
        let (date, time, week, number) = match region {
            Some(R::Germany | R::Austria) => (D::De, T::H24, W::Monday, N::Comma),
            Some(R::Switzerland) => (D::De, T::H24, W::Monday, N::Apostrophe),
            Some(R::France) => (D::Dmy, T::H24, W::Monday, N::Space),
            Some(R::Italy | R::Netherlands | R::Spain) => (D::Dmy, T::H24, W::Monday, N::Comma),
            Some(R::UnitedKingdom) => (D::Dmy, T::H24, W::Monday, N::Point),
            Some(R::UnitedStates) => (D::Us, T::H12, W::Sunday, N::Point),
            None if lang == Language::German => (D::De, T::H24, W::Monday, N::Comma),
            None => (D::Iso, T::H24, W::Monday, N::Point),
        };
        Self {
            date,
            time,
            week,
            number,
        }
    }
}

/// Time zone as an IANA name known to the embedded time zone database, in
/// its spelling ("europe/berlin" becomes "Europe/Berlin", as `Intl` names
/// it). While `time_zone_auto` applies, the web app reports the browser's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Zone(String);

impl Zone {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Zone {
    fn default() -> Self {
        Self(DEFAULT_TZ.to_owned())
    }
}

impl TryFrom<String> for Zone {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        // Longer than any name in the database; guards against arbitrary input.
        let name = (value.len() <= 64)
            .then(|| TimeZone::get(&value).ok())
            .flatten()
            .and_then(|zone| zone.iana_name().map(str::to_owned));
        name.map(Self).ok_or("unknown time zone")
    }
}

impl From<Zone> for String {
    fn from(zone: Zone) -> Self {
        zone.0
    }
}

/// How long an action can be undone, in seconds: 3, 5 or 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct UndoDelay(u8);

impl Default for UndoDelay {
    fn default() -> Self {
        Self(5)
    }
}

impl TryFrom<u8> for UndoDelay {
    type Error = &'static str;

    fn try_from(seconds: u8) -> Result<Self, Self::Error> {
        matches!(seconds, 3 | 5 | 10)
            .then_some(Self(seconds))
            .ok_or("3, 5 or 10 seconds")
    }
}

impl From<UndoDelay> for u8 {
    fn from(delay: UndoDelay) -> Self {
        delay.0
    }
}

/// The settings document. Missing fields take the default, unknown ones make
/// reading fail (then [`load`] takes over field by field).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    // Appearance
    /// `None`: no choice yet (the web app follows the system).
    pub mode: Option<ColorScheme>,
    /// `None`: the brand color.
    pub accent: Option<Accent>,
    pub font_size: FontSize,
    pub density: Density,
    pub animations: bool,
    // Language and region
    pub lang: Language,
    /// `None`: not set.
    pub region: Option<Region>,
    pub time_zone_auto: bool,
    pub time_zone: Zone,
    /// `None` for each format: automatic ([`Formats::of`]).
    pub date_format: Option<DateFormat>,
    pub time_format: Option<TimeFormat>,
    pub week_start: Option<WeekStart>,
    pub number_format: Option<NumberFormat>,
    // General
    /// Ask before deleting.
    pub confirm_delete: bool,
    pub undo_delay: UndoDelay,
    /// Keyboard shortcuts on.
    pub shortcuts: bool,
    /// Developer mode: the apps' developer tools.
    pub developer: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: None,
            accent: None,
            font_size: FontSize::default(),
            density: Density::default(),
            animations: true,
            lang: Language::default(),
            region: None,
            time_zone_auto: true,
            time_zone: Zone::default(),
            date_format: None,
            time_format: None,
            week_start: None,
            number_format: None,
            confirm_delete: true,
            undo_delay: UndoDelay::default(),
            shortcuts: true,
            developer: false,
        }
    }
}

/// The formats a new region sets back to automatic.
const FORMAT_FIELDS: [&str; 4] = ["date_format", "time_format", "week_start", "number_format"];

/// Fields of a change that could not be applied: name and code (`unknown`,
/// `invalid`).
#[derive(Debug, PartialEq, Eq)]
pub struct Rejected(pub Vec<(String, &'static str)>);

impl Settings {
    /// The settings with `changes` applied. Every field is checked; a single
    /// bad one rejects the whole change. `null` clears the fields that may be
    /// empty (`mode`, `accent`, `region`, the formats). A region other than
    /// the current one sets the formats the change does not name back to
    /// automatic, so that the new region's apply.
    pub fn apply(&self, changes: &Map<String, Value>) -> Result<Self, Rejected> {
        let mut rejected = Vec::new();
        let mut next = self.merge(changes, |field, code| {
            rejected.push((field.to_owned(), code));
        });
        if !rejected.is_empty() {
            return Err(Rejected(rejected));
        }
        if next.region != self.region {
            for field in FORMAT_FIELDS {
                if !changes.contains_key(field) {
                    next.clear_format(field);
                }
            }
        }
        Ok(next)
    }

    fn clear_format(&mut self, field: &str) {
        match field {
            "date_format" => self.date_format = None,
            "time_format" => self.time_format = None,
            "week_start" => self.week_start = None,
            "number_format" => self.number_format = None,
            _ => {}
        }
    }

    /// Takes over every field of `changes` that fits; reports the others.
    fn merge(
        &self,
        changes: &Map<String, Value>,
        mut reject: impl FnMut(&str, &'static str),
    ) -> Self {
        fn set<T: DeserializeOwned>(slot: &mut T, value: &Value) -> bool {
            T::deserialize(value).map(|parsed| *slot = parsed).is_ok()
        }
        let mut next = self.clone();
        for (field, value) in changes {
            let n = &mut next;
            let ok = match field.as_str() {
                "mode" => set(&mut n.mode, value),
                "accent" => set(&mut n.accent, value),
                "font_size" => set(&mut n.font_size, value),
                "density" => set(&mut n.density, value),
                "animations" => set(&mut n.animations, value),
                "lang" => set(&mut n.lang, value),
                "region" => set(&mut n.region, value),
                "time_zone_auto" => set(&mut n.time_zone_auto, value),
                "time_zone" => set(&mut n.time_zone, value),
                "date_format" => set(&mut n.date_format, value),
                "time_format" => set(&mut n.time_format, value),
                "week_start" => set(&mut n.week_start, value),
                "number_format" => set(&mut n.number_format, value),
                "confirm_delete" => set(&mut n.confirm_delete, value),
                "undo_delay" => set(&mut n.undo_delay, value),
                "shortcuts" => set(&mut n.shortcuts, value),
                "developer" => set(&mut n.developer, value),
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

    /// Color scheme for the apps (`theme` in `prefs`).
    pub fn theme(&self) -> ColorScheme {
        self.mode.unwrap_or(ColorScheme::System)
    }

    /// The formats that apply: the chosen ones, the others automatic.
    pub fn formats(&self, lang: Language) -> Formats {
        let auto = Formats::of(self.region, lang);
        Formats {
            date: self.date_format.unwrap_or(auto.date),
            time: self.time_format.unwrap_or(auto.time),
            week: self.week_start.unwrap_or(auto.week),
            number: self.number_format.unwrap_or(auto.number),
        }
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

impl Stored {
    /// The formats that apply ([`Settings::formats`]).
    pub fn formats(&self) -> Formats {
        self.settings.formats(self.lang)
    }

    /// Time zone, formats and language of the user's mails.
    pub fn format(&self) -> Format {
        let formats = self.formats();
        Format::new(
            self.settings.time_zone.as_str(),
            formats.date,
            formats.time,
            self.lang,
        )
    }

    /// What an app's backend gets in the internal API and `user.updated`
    /// (contract § 7): time zone, region and the formats that apply.
    pub fn app_prefs(&self) -> UserPrefs {
        let formats = self.formats();
        UserPrefs {
            time_zone: self.settings.time_zone.as_str().to_owned(),
            region: self.settings.region.map(Region::code),
            date_format: name_of(formats.date),
            time_format: name_of(formats.time),
            week_start: name_of(formats.week),
            number_format: name_of(formats.number),
        }
    }

    /// Everything the apps apply in the browser (`prefs` of `GET session`,
    /// contract § 6): the formats resolved, the accent only while the
    /// branding lets users choose one (`user_accent`), the language as
    /// chosen (`system` follows the browser).
    pub fn browser_prefs(&self, user_accent: bool) -> Value {
        let s = &self.settings;
        let formats = self.formats();
        json!({
            "theme": s.theme().as_str(),
            "accent": s.accent.filter(|_| user_accent).map(Accent::id),
            "font_size": s.font_size,
            "density": s.density,
            "animations": s.animations,
            "language": s.lang.code(),
            "region": s.region,
            "time_zone": s.time_zone.as_str(),
            "time_zone_auto": s.time_zone_auto,
            "date_format": formats.date,
            "time_format": formats.time,
            "week_start": formats.week,
            "number_format": formats.number,
            "confirm_delete": s.confirm_delete,
            "undo_delay": s.undo_delay,
            "shortcuts": s.shortcuts,
            "developer": s.developer,
        })
    }
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
/// version; `None` if another one applies by now. If the user's language
/// changes with it, the apps get `user.updated` (`locale`).
pub fn save(
    conn: &Connection,
    user_id: &str,
    version: i64,
    settings: &Settings,
) -> anyhow::Result<Option<i64>> {
    let json = serde_json::to_string(settings)?;
    events::tracking_user(conn, user_id, |conn| {
        Ok(user_settings::save(conn, user_id, &json, version)?)
    })
}

/// A user's language for the server's texts ([`Stored::lang`]).
pub fn lang_of(conn: &Connection, user_id: &str) -> anyhow::Result<Language> {
    Ok(load(conn, user_id)?.lang)
}

/// Time zone, formats and language of a user's mails ([`Stored::format`]).
pub fn format_of(conn: &Connection, user_id: &str) -> anyhow::Result<Format> {
    Ok(load(conn, user_id)?.format())
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
                json!({ "lang": "fr", "mode": null, "density": "dense", "accent": 3,
                        "zoom": 2, "undo_delay": 4, "time_zone": "Mars/Olympus",
                        "animations": null, "number_format": "1 234,56" }),
            ))
            .unwrap_err();
        let mut fields = rejected.0;
        fields.sort();
        assert_eq!(
            fields,
            [
                ("accent".to_owned(), "invalid"),
                ("animations".to_owned(), "invalid"),
                ("density".to_owned(), "invalid"),
                ("lang".to_owned(), "invalid"),
                ("number_format".to_owned(), "invalid"),
                ("time_zone".to_owned(), "invalid"),
                ("undo_delay".to_owned(), "invalid"),
                ("zoom".to_owned(), "unknown"),
            ]
        );
        assert!(base.apply(&changes(json!({ "lang": null }))).is_err());
    }

    #[test]
    fn apply_takes_every_shared_setting() {
        let next = Settings::default()
            .apply(&changes(json!({
                "font_size": "large", "density": "compact", "animations": false,
                "region": "FR", "time_zone_auto": false, "time_zone": "america/new_york",
                "date_format": "iso", "time_format": "12h", "week_start": "sunday",
                "number_format": "1\u{2019}234.56", "confirm_delete": false, "undo_delay": 10,
                "shortcuts": false, "developer": true,
            })))
            .unwrap();
        assert_eq!(next.font_size, FontSize::Large);
        assert_eq!(next.density, Density::Compact);
        assert!(!next.animations && !next.time_zone_auto && !next.confirm_delete);
        assert!(!next.shortcuts && next.developer);
        assert_eq!(next.region, Some(Region::France));
        // In the database's spelling.
        assert_eq!(next.time_zone.as_str(), "America/New_York");
        assert_eq!(u8::from(next.undo_delay), 10);
        // Named in the same change as the region: kept.
        assert_eq!(
            next.formats(Language::English),
            Formats {
                date: DateFormat::Iso,
                time: TimeFormat::H12,
                week: WeekStart::Sunday,
                number: NumberFormat::Apostrophe,
            }
        );
        let doc = serde_json::to_value(&next).unwrap();
        assert_eq!(doc["number_format"], "1\u{2019}234.56");
        assert_eq!(doc["undo_delay"], 10);
        assert_eq!(Settings::deserialize(&doc).unwrap(), next);
    }

    #[test]
    fn a_new_region_sets_the_formats_back_to_automatic() {
        let chosen = Settings::default()
            .apply(&changes(
                json!({ "region": "DE", "time_format": "12h", "date_format": "iso" }),
            ))
            .unwrap();
        assert_eq!(chosen.formats(Language::English).time, TimeFormat::H12);
        // The same region again keeps the own choice.
        let same = chosen.apply(&changes(json!({ "region": "DE" }))).unwrap();
        assert_eq!(same.time_format, Some(TimeFormat::H12));
        // Another region: everything automatic, except what the change names.
        let us = chosen
            .apply(&changes(json!({ "region": "US", "date_format": "de" })))
            .unwrap();
        assert_eq!(
            (
                us.date_format,
                us.time_format,
                us.week_start,
                us.number_format
            ),
            (Some(DateFormat::De), None, None, None)
        );
        assert_eq!(
            us.formats(Language::German),
            Formats {
                date: DateFormat::De,
                time: TimeFormat::H12,
                week: WeekStart::Sunday,
                number: NumberFormat::Point,
            }
        );
        let none = us.apply(&changes(json!({ "region": null }))).unwrap();
        assert_eq!(none.region, None);
        assert_eq!(none.formats(Language::German).number, NumberFormat::Comma);
        assert_eq!(none.formats(Language::English).date, DateFormat::Iso);
    }

    #[test]
    fn regions_pick_their_formats() {
        let of = |region| Formats::of(Some(region), Language::English);
        assert_eq!(of(Region::Austria).date, DateFormat::De);
        assert_eq!(of(Region::Switzerland).number, NumberFormat::Apostrophe);
        assert_eq!(of(Region::France).number, NumberFormat::Space);
        assert_eq!(of(Region::Spain).date, DateFormat::Dmy);
        assert_eq!(of(Region::UnitedKingdom).number, NumberFormat::Point);
        let us = of(Region::UnitedStates);
        assert_eq!(
            (us.date, us.time, us.week),
            (DateFormat::Us, TimeFormat::H12, WeekStart::Sunday)
        );
        assert_eq!(Region::UnitedKingdom.code(), "GB");
    }

    #[test]
    fn prefs_for_the_apps() {
        let settings = Settings::default()
            .apply(&changes(json!({
                "mode": "dark", "accent": "sage", "region": "AT", "time_zone": "Europe/Vienna",
                "lang": "system", "time_format": "12h",
            })))
            .unwrap();
        let stored = Stored {
            version: 4,
            settings,
            rewrite: false,
            lang: Language::German,
        };
        let backend = stored.app_prefs();
        assert_eq!(backend.time_zone, "Europe/Vienna");
        assert_eq!(backend.region.as_deref(), Some("AT"));
        assert_eq!(
            [
                backend.date_format.as_str(),
                &backend.time_format,
                &backend.week_start,
                &backend.number_format,
            ],
            ["de", "12h", "monday", "1.234,56"]
        );
        let browser = stored.browser_prefs(true);
        assert_eq!(browser["theme"], "dark");
        assert_eq!(browser["accent"], "sage");
        assert_eq!(browser["language"], "system");
        assert_eq!(browser["date_format"], "de");
        assert_eq!(browser["undo_delay"], 5);
        assert_eq!(browser.as_object().unwrap().len(), 17);
        // The branding fixes the accent.
        assert_eq!(stored.browser_prefs(false)["accent"], Value::Null);
        let mail = stored.format();
        assert_eq!(mail.lang(), Language::German);
        assert_eq!(
            mail.time("2026-10-30T13:07:00Z".parse().unwrap()),
            "2:07 PM"
        );
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
            region: Some(Region::Germany),
            ..Settings::default()
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
            (
                &id,
                r#"{"lang":"de","mode":"dim","future":1,"font_size":"large","undo_delay":7}"#,
            ),
        )
        .unwrap();
        let stored = load(&conn, &id).unwrap();
        assert_eq!(stored.version, 3);
        assert_eq!(stored.settings.lang, Language::German);
        assert_eq!(stored.settings.mode, None);
        assert_eq!(stored.settings.font_size, FontSize::Large);
        assert_eq!(stored.settings.undo_delay, UndoDelay::default());
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
