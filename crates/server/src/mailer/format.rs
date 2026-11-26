//! Details in mails – dates, times and names – in the recipient's time zone,
//! with the words of their language (month names, "Today") from the locale
//! files.
//!
//! Taken over from Roedy Mail (`crates/server/src/mailer/format.rs`) without
//! the date and time formats of Mail's settings, sizes and intervals: the hub
//! writes dates spelled out ("30 October 2026") and times on the 24-hour
//! clock, in the default time zone until the settings offer one.

use jiff::{Timestamp, tz::TimeZone};

pub use crate::api::sessions::short_ip;
use crate::{
    i18n::{t, tf},
    settings::Language,
};

/// Default time zone, as in Roedy Mail.
pub const DEFAULT_TZ: &str = "Europe/Berlin";

/// Time zone and language of a recipient.
#[derive(Debug, Clone)]
pub struct Format {
    tz: TimeZone,
    lang: Language,
}

impl Default for Format {
    fn default() -> Self {
        Self {
            tz: named_zone(DEFAULT_TZ),
            lang: Language::default(),
        }
    }
}

impl Format {
    /// Time zone as IANA name (unknown → default) and language.
    pub fn new(tz: &str, lang: Language) -> Self {
        Self {
            tz: named_zone(tz),
            lang,
        }
    }

    /// The same time zone in `lang`: for recipients without settings
    /// (invitations).
    #[must_use]
    pub fn with_lang(self, lang: Language) -> Self {
        Self { lang, ..self }
    }

    /// Language of the recipient (templates, subjects, texts).
    pub fn lang(&self) -> Language {
        self.lang
    }

    /// Date, e.g. "30 October 2026".
    pub fn date(&self, at: Timestamp) -> String {
        let at = at.to_zoned_datetime(self.tz.clone());
        tf(
            self.lang,
            "format.date",
            &[
                ("day", &at.day()),
                (
                    "month",
                    &t(self.lang, &format!("format.month.{}", at.month())),
                ),
                ("year", &at.year()),
            ],
        )
    }

    /// Time, e.g. "15:57", with the suffix of the language if it has one.
    pub fn time(&self, at: Timestamp) -> String {
        let at = at.to_zoned_datetime(self.tz.clone());
        tf(
            self.lang,
            "format.time_24h",
            &[("time", &format!("{:02}:{:02}", at.hour(), at.minute()))],
        )
    }

    /// Point in time: "Today, 15:57", on another day "23 October 2026,
    /// 14:28" (day in the recipient's time zone, compared with `now`).
    pub fn when(&self, at: Timestamp, now: Timestamp) -> String {
        let today = at.to_zoned_datetime(self.tz.clone()).date()
            == now.to_zoned_datetime(self.tz.clone()).date();
        let day = if today {
            t(self.lang, "format.today").to_owned()
        } else {
            self.date(at)
        };
        format!("{day}, {}", self.time(at))
    }
}

/// Time zone as IANA name; unknown ones get the default (and without the
/// default UTC).
pub fn named_zone(name: &str) -> TimeZone {
    TimeZone::get(name)
        .or_else(|_| TimeZone::get(DEFAULT_TZ))
        .unwrap_or(TimeZone::UTC)
}

/// Display name in reading order: "Example, Max" becomes "Max Example".
pub fn full_name(name: &str) -> String {
    match name.split_once(',') {
        Some((last, rest)) => {
            // Only the part up to the next comma.
            let first = rest.split(',').next().unwrap_or_default();
            format!("{} {}", first.trim(), last.trim())
                .trim()
                .to_owned()
        }
        None => name.trim().to_owned(),
    }
}

/// First name for the greeting ("Hi Max,").
pub fn first_name(name: &str) -> Option<String> {
    full_name(name).split_whitespace().next().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn dates_in_the_recipients_zone() {
        let format = Format::default();
        // 30.10. 23:30 UTC is already the 31st in Berlin.
        assert_eq!(format.date(at("2026-10-30T23:30:00Z")), "31 October 2026");
        assert_eq!(format.date(at("2026-03-01T12:00:00Z")), "1 March 2026");
        let new_york = Format::new("America/New_York", Language::English);
        assert_eq!(new_york.date(at("2026-10-31T02:00:00Z")), "30 October 2026");
    }

    #[test]
    fn times_follow_daylight_saving() {
        let format = Format::default();
        // Daylight saving time until 25.10.2026, CET afterwards.
        assert_eq!(format.time(at("2026-10-24T13:57:00Z")), "15:57");
        assert_eq!(format.time(at("2026-10-26T13:57:00Z")), "14:57");
    }

    #[test]
    fn today_or_date() {
        let format = Format::default();
        let now = at("2026-10-24T20:00:00Z");
        assert_eq!(format.when(at("2026-10-24T13:57:00Z"), now), "Today, 15:57");
        assert_eq!(
            format.when(at("2026-10-23T12:28:00Z"), now),
            "23 October 2026, 14:28"
        );
        // 22:30 UTC is already the next day in Berlin.
        assert_eq!(
            format.when(at("2026-10-24T22:30:00Z"), now),
            "25 October 2026, 00:30"
        );
    }

    #[test]
    fn unknown_zone_falls_back() {
        let format = Format::new("Mars/Olympus", Language::English);
        assert_eq!(format.time(at("2026-10-24T13:57:00Z")), "15:57");
    }

    #[test]
    fn names_in_reading_order() {
        assert_eq!(full_name("Example, Max"), "Max Example");
        assert_eq!(full_name(" Max Example "), "Max Example");
        assert_eq!(first_name("Example, Max"), Some("Max".into()));
        assert_eq!(first_name("Jonas Example"), Some("Jonas".into()));
        assert_eq!(first_name("  "), None);
        assert_eq!(full_name("Example,"), "Example");
    }

    #[test]
    fn english_is_the_default() {
        assert_eq!(Format::default().lang(), Language::English);
    }

    /// German uses the same digits with its own words.
    #[test]
    fn german_words_with_the_same_digits() {
        let de = Language::German;
        let format = Format::default().with_lang(de);
        let now = at("2026-10-24T20:00:00Z");
        let october = t(de, "format.month.10");
        assert_ne!(october, t(Language::English, "format.month.10"));
        assert_eq!(
            format.date(at("2026-10-23T12:28:00Z")),
            tf(
                de,
                "format.date",
                &[("day", &23), ("month", &october), ("year", &2026)]
            )
        );
        let time = tf(de, "format.time_24h", &[("time", &"15:57")]);
        assert_ne!(time, "15:57");
        assert_eq!(format.time(at("2026-10-24T13:57:00Z")), time);
        assert_eq!(
            format.when(at("2026-10-24T13:57:00Z"), now),
            format!("{}, {time}", t(de, "format.today"))
        );
    }
}
