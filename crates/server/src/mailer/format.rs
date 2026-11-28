//! Details in mails – dates, times and names – in the recipient's time zone
//! and formats (`time_zone`, `date_format`, `time_format` of their settings,
//! `crate::settings`), with the words of their language (month names,
//! "Today") from the locale files. For recipients without an account
//! (invitations) the default formats apply in the inviter's language.
//!
//! Taken over from Roedy Mail (`crates/server/src/mailer/format.rs`) without
//! sizes and intervals, plus the date format `dmy`.

use jiff::{Timestamp, tz::TimeZone};

pub use crate::api::sessions::short_ip;
use crate::{
    i18n::{t, tf},
    settings::{DateFormat, Language, TimeFormat},
};

/// Default time zone, as in Roedy Mail.
pub const DEFAULT_TZ: &str = "Europe/Berlin";

/// Time zone, formats and language of a recipient.
#[derive(Debug, Clone)]
pub struct Format {
    tz: TimeZone,
    date: DateFormat,
    time: TimeFormat,
    lang: Language,
}

impl Default for Format {
    /// The default zone, the date spelled out and the 24-hour clock, in
    /// English.
    fn default() -> Self {
        Self {
            tz: named_zone(DEFAULT_TZ),
            date: DateFormat::De,
            time: TimeFormat::H24,
            lang: Language::default(),
        }
    }
}

impl Format {
    /// Time zone as IANA name (unknown → default), formats and language.
    pub fn new(tz: &str, date: DateFormat, time: TimeFormat, lang: Language) -> Self {
        Self {
            tz: named_zone(tz),
            date,
            time,
            lang,
        }
    }

    /// The same formats in `lang`: for recipients without settings
    /// (invitations).
    #[must_use]
    pub fn with_lang(self, lang: Language) -> Self {
        Self { lang, ..self }
    }

    /// Language of the recipient (templates, subjects, texts).
    pub fn lang(&self) -> Language {
        self.lang
    }

    /// Date: `de` spelled out with the month's name ("30 October 2026", in
    /// German "30. Oktober 2026"), `dmy` "30/10/2026", `iso` "2026-10-30",
    /// `us` "10/30/2026".
    pub fn date(&self, at: Timestamp) -> String {
        let at = at.to_zoned_datetime(self.tz.clone());
        let (year, month, day) = (at.year(), at.month(), at.day());
        match self.date {
            DateFormat::De => tf(
                self.lang,
                "format.date",
                &[
                    ("day", &day),
                    ("month", &t(self.lang, &format!("format.month.{month}"))),
                    ("year", &year),
                ],
            ),
            DateFormat::Dmy => format!("{day:02}/{month:02}/{year:04}"),
            DateFormat::Iso => format!("{year:04}-{month:02}-{day:02}"),
            DateFormat::Us => format!("{month:02}/{day:02}/{year:04}"),
        }
    }

    /// Time: "15:57" with the suffix of the language if it has one, or
    /// "3:57 PM" in every language.
    pub fn time(&self, at: Timestamp) -> String {
        let at = at.to_zoned_datetime(self.tz.clone());
        let (hour, minute) = (at.hour(), at.minute());
        match self.time {
            TimeFormat::H24 => tf(
                self.lang,
                "format.time_24h",
                &[("time", &format!("{hour:02}:{minute:02}"))],
            ),
            TimeFormat::H12 => {
                let suffix = if hour < 12 { "AM" } else { "PM" };
                let hour = match hour % 12 {
                    0 => 12,
                    hour => hour,
                };
                format!("{hour}:{minute:02} {suffix}")
            }
        }
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
        let new_york = Format::new(
            "America/New_York",
            DateFormat::De,
            TimeFormat::H24,
            Language::English,
        );
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
        let format = Format::new(
            "Mars/Olympus",
            DateFormat::De,
            TimeFormat::H24,
            Language::English,
        );
        assert_eq!(format.time(at("2026-10-24T13:57:00Z")), "15:57");
    }

    #[test]
    fn formats_in_digits_and_on_the_12_hour_clock() {
        let at_ = at("2026-10-30T13:07:00Z");
        let with = |date, time| Format::new("Europe/London", date, time, Language::German);
        assert_eq!(
            with(DateFormat::Dmy, TimeFormat::H12).date(at_),
            "30/10/2026"
        );
        assert_eq!(
            with(DateFormat::Iso, TimeFormat::H12).date(at_),
            "2026-10-30"
        );
        assert_eq!(
            with(DateFormat::Us, TimeFormat::H12).date(at_),
            "10/30/2026"
        );
        assert_eq!(with(DateFormat::Us, TimeFormat::H12).time(at_), "1:07 PM");
        let midnight = at("2026-10-30T00:05:00Z");
        assert_eq!(
            with(DateFormat::Us, TimeFormat::H12).time(midnight),
            "12:05 AM"
        );
        let noon = at("2026-10-30T12:00:00Z");
        assert_eq!(with(DateFormat::Us, TimeFormat::H12).time(noon), "12:00 PM");
        // The 24-hour clock keeps the language's suffix.
        assert_eq!(
            with(DateFormat::Iso, TimeFormat::H24).time(at_),
            tf(Language::German, "format.time_24h", &[("time", &"13:07")])
        );
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
