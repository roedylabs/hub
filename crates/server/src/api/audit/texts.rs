//! Audit log texts: icon, person, action and details of an entry are built on
//! read from the action key and details, in the administrator's language –
//! one source for list, search and export. The wording lives in the locale
//! files under `audit.*` (`crate::i18n`).
//!
//! `who` and `act` follow each other, so `act` starts with a space (" signed
//! in") or a colon (": Sign-in denied"); `meta` is a detail such as the
//! shortened IP address of a sign-in.
//!
//! An unknown key (say after a downgrade) or missing details show the key
//! instead of the text, so no entry disappears.
//!
//! Taken over from Roedy Mail (`crates/server/src/api/audit/texts.rs`) with
//! the keys the hub writes, plus `auth.reauth` and the category `apps`.

use std::fmt::Display;

use serde_json::Value;

use crate::{
    api::sessions::short_ip,
    db::repo::audit_log::{Category, Row},
    i18n::{plural, t, tf},
    settings::Language,
};

/// An entry to display and search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// Icon as a Tabler Icons class (`ti-…`, registry of the web app).
    pub icon: &'static str,
    pub who: String,
    pub act: String,
    pub meta: String,
    /// Highlighted as a warning.
    pub warn: bool,
}

impl Shown {
    /// Text the search matches.
    pub fn searchable(&self, category: Category, lang: Language) -> String {
        format!(
            "{}{} {} {}",
            self.who,
            self.act,
            self.meta,
            label(category, lang)
        )
    }
}

/// Column headers of the export.
pub fn csv_head(lang: Language) -> [&'static str; 5] {
    ["time", "category", "person", "action", "details"]
        .map(|column| t(lang, &format!("audit.csv.{column}")))
}

/// Name of the category.
pub fn label(category: Category, lang: Language) -> &'static str {
    t(lang, &format!("audit.category.{}", category.as_str()))
}

/// An entry from action key and details.
pub fn render(row: &Row, lang: Language) -> Shown {
    let line = known(row, lang)
        .unwrap_or_else(|| Line::new(fallback_icon(row.category), format!(": {}", row.action)));
    Shown {
        icon: line.icon,
        who: row
            .actor_name
            .clone()
            .unwrap_or_else(|| label(Category::System, lang).to_owned()),
        act: line.act,
        meta: line.meta.or_else(|| login_ip(row)).unwrap_or_default(),
        warn: line.warn,
    }
}

/// Sign-ins show the shortened IP address; other entries have no detail.
fn login_ip(row: &Row) -> Option<String> {
    (row.category == Category::Login)
        .then(|| row.ip.as_deref()?.parse().ok().map(short_ip))
        .flatten()
}

fn fallback_icon(category: Category) -> &'static str {
    match category {
        Category::Login => "ti-login",
        Category::Users => "ti-users",
        Category::Apps => "ti-apps",
        Category::Settings => "ti-settings",
        Category::System => "ti-server",
    }
}

/// Text of a key without person and detail.
struct Line {
    icon: &'static str,
    act: String,
    meta: Option<String>,
    warn: bool,
}

impl Line {
    fn new(icon: &'static str, act: impl Into<String>) -> Self {
        Self {
            icon,
            act: act.into(),
            meta: None,
            warn: false,
        }
    }

    fn warn(mut self) -> Self {
        self.warn = true;
        self
    }

    fn meta(mut self, meta: Option<String>) -> Self {
        self.meta = meta;
        self
    }
}

/// Details of an entry.
#[derive(Clone, Copy)]
struct Details<'a>(Option<&'a Value>);

impl<'a> Details<'a> {
    fn text(self, key: &str) -> Option<&'a str> {
        self.0?.get(key)?.as_str()
    }

    fn number(self, key: &str) -> Option<u64> {
        self.0?.get(key)?.as_u64()
    }
}

/// Role within a sentence ("the user role").
fn role(lang: Language, role: &str) -> Option<&'static str> {
    matches!(role, "admin" | "user").then(|| t(lang, &format!("audit.role.{role}")))
}

/// Role after "as" ("invited … as a user").
fn role_as(lang: Language, role: &str) -> Option<&'static str> {
    matches!(role, "admin" | "user").then(|| t(lang, &format!("audit.role_as.{role}")))
}

/// Retention period: a year or a number of days.
fn keep(lang: Language, days: u64) -> String {
    match days {
        365 => t(lang, "audit.keep_year").to_owned(),
        days => plural(lang, "duration.days", days, &[]),
    }
}

/// Icon, text and highlight of a key; `None` for an unknown key or missing
/// details.
fn known(row: &Row, lang: Language) -> Option<Line> {
    let p = Details(row.details.as_ref());
    let action = row.action.as_str();
    // Text of the action, `audit.<action>[.<variant>]`.
    let key = |variant: Option<&str>| match variant {
        Some(variant) => format!("audit.{action}.{variant}"),
        None => format!("audit.{action}"),
    };
    let act = |variant: Option<&str>, args: &[(&str, &dyn Display)]| tf(lang, &key(variant), args);
    let plain = |icon| Line::new(icon, act(None, &[]));
    let with = |icon, name: &str, value: &dyn Display| Line::new(icon, act(None, &[(name, value)]));
    Some(match action {
        // Sign-in
        "auth.login" | "auth.reauth" if p.text("method") == Some("recovery") => {
            Line::new("ti-lifebuoy", act(Some("recovery"), &[]))
        }
        "auth.login" => plain("ti-login"),
        "auth.reauth" => plain("ti-shield-check"),
        "auth.login_failed" => Line::new(
            "ti-alert-triangle",
            act(
                match p.text("reason") {
                    Some(reason @ ("locked" | "password" | "totp" | "recovery")) => Some(reason),
                    _ => None,
                },
                &[],
            ),
        )
        .warn(),
        "auth.logout" => plain("ti-logout"),
        "auth.password_change" | "auth.password_forgot" | "auth.password_reset" => plain("ti-key"),
        "auth.email_change_request" | "auth.email_change_resend" => {
            with("ti-mail-forward", "email", &p.text("email")?)
        }
        "auth.email_change_confirm" => with("ti-mail-check", "email", &p.text("email")?),
        "auth.email_change_discard" => plain("ti-mail-off"),
        "auth.profile_change" => plain("ti-user-edit"),
        "auth.recovery_codes" => plain("ti-lifebuoy"),
        "auth.session_revoke" => plain("ti-device-desktop-off"),
        "auth.sessions_revoke" => Line::new(
            "ti-devices",
            plural(lang, &key(None), p.number("sessions_ended")?, &[]),
        ),
        "auth.totp_add" => with("ti-shield-plus", "name", &p.text("name")?),
        "auth.totp_remove" => with("ti-shield-minus", "name", &p.text("name")?),
        // Users
        "users.invite" => Line::new(
            "ti-user-plus",
            act(
                None,
                &[
                    ("email", &p.text("email")?),
                    ("role", &role_as(lang, p.text("role")?)?),
                ],
            ),
        ),
        "users.invite_accept" => plain("ti-user-check"),
        "users.invite_renew" => with("ti-calendar-plus", "email", &p.text("email")?),
        "users.invite_request" => plain("ti-user-plus"),
        "users.invite_resend" => with("ti-send", "email", &p.text("email")?),
        "users.invite_revoke" => with("ti-user-minus", "email", &p.text("email")?),
        "users.invite_role" => Line::new(
            "ti-user-cog",
            act(
                None,
                &[
                    ("name", &p.text("email")?),
                    ("role", &role(lang, p.text("role")?)?),
                ],
            ),
        ),
        "users.lock" => with("ti-lock", "name", &p.text("name")?),
        "users.unlock" => with("ti-lock-open", "name", &p.text("name")?),
        "users.remove" => with("ti-user-minus", "name", &p.text("name")?),
        "users.delete_self" => plain("ti-user-minus"),
        "users.role" => Line::new(
            "ti-user-cog",
            act(
                None,
                &[
                    ("name", &p.text("name")?),
                    ("role", &role(lang, p.text("role")?)?),
                ],
            ),
        ),
        "users.mfa_reset" => with("ti-shield-off", "name", &p.text("name")?),
        // Settings
        "org.update" => plain("ti-building"),
        "branding.update" => plain("ti-brush"),
        "system_mail.update" => plain("ti-mail"),
        "audit.retention" => with("ti-history", "keep", &keep(lang, p.number("days")?)),
        // System
        "auth.reset_admin" => with("ti-user-shield", "user", &p.text("user")?).warn(),
        "setup.complete" => plain("ti-rocket"),
        "audit.export" => plain("ti-download").meta(
            p.number("count")
                .map(|n| plural(lang, "audit.entries", n, &[])),
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn row(action: &str, category: Category, details: Value) -> Row {
        Row {
            id: 1,
            at: "2026-11-05T09:26:00.000Z".into(),
            category,
            action: action.into(),
            actor_name: Some("Anna Example".into()),
            ip: None,
            details: Some(details),
        }
    }

    fn shown_en(action: &str, category: Category, details: Value) -> Shown {
        render(&row(action, category, details), Language::English)
    }

    #[test]
    fn sign_in_with_ip_and_warnings() {
        let mut login = row(
            "auth.login",
            Category::Login,
            json!({ "method": "password" }),
        );
        login.ip = Some("198.51.100.56".into());
        let line = render(&login, Language::English);
        assert_eq!(line.icon, "ti-login");
        assert_eq!(line.who, "Anna Example");
        assert_eq!(line.act, " signed in");
        assert_eq!(line.meta, "198.51.x.x");
        assert!(!line.warn);

        let locked = shown_en(
            "auth.login_failed",
            Category::Login,
            json!({ "reason": "locked" }),
        );
        assert_eq!(locked.icon, "ti-alert-triangle");
        assert!(locked.warn);
        let reset = shown_en(
            "auth.reset_admin",
            Category::System,
            json!({ "user": "Anna Example" }),
        );
        assert!(reset.warn);
    }

    /// All action keys the hub writes, with their category. A new key
    /// belongs here and in both locale files.
    const ACTIONS: [(&str, Category); 39] = {
        use Category::{Login, Settings, System, Users};
        [
            ("auth.login", Login),
            ("auth.login_failed", Login),
            ("auth.logout", Login),
            ("auth.reauth", Login),
            ("auth.password_change", Login),
            ("auth.password_forgot", Login),
            ("auth.password_reset", Login),
            ("auth.email_change_request", Login),
            ("auth.email_change_resend", Login),
            ("auth.email_change_confirm", Login),
            ("auth.email_change_discard", Login),
            ("auth.profile_change", Login),
            ("auth.recovery_codes", Login),
            ("auth.session_revoke", Login),
            ("auth.sessions_revoke", Login),
            ("auth.totp_add", Login),
            ("auth.totp_remove", Login),
            ("users.invite", Users),
            ("users.invite_accept", Users),
            ("users.invite_renew", Users),
            ("users.invite_request", Users),
            ("users.invite_resend", Users),
            ("users.invite_revoke", Users),
            ("users.invite_role", Users),
            ("users.lock", Users),
            ("users.unlock", Users),
            ("users.remove", Users),
            ("users.delete_self", Users),
            ("users.role", Users),
            ("users.mfa_reset", Users),
            ("org.update", Settings),
            ("branding.update", Settings),
            ("system_mail.update", Settings),
            ("audit.retention", Settings),
            ("auth.reset_admin", System),
            ("setup.complete", System),
            ("audit.export", System),
            // Keys with a variant per detail, once more for the other branch.
            ("auth.login", Login),
            ("auth.reauth", Login),
        ]
    };

    /// Details with which every key has a text.
    fn all_details() -> Value {
        json!({
            "method": "password", "reason": "password", "email": "a@example.com",
            "sessions_ended": 2, "name": "Mia Example", "role": "admin",
            "user": "Anna Example", "days": 365, "count": 2,
        })
    }

    /// `all_details` with the other branches of the conditions.
    fn other_details() -> Value {
        json!({
            "method": "recovery", "reason": "locked", "email": "a@example.com",
            "sessions_ended": 1, "name": "Mia Example", "role": "user",
            "user": "Anna Example", "days": 30, "count": 1,
        })
    }

    #[test]
    fn lines_read_like_sentences() {
        use Category::{Login, Settings, System, Users};
        let email = json!({ "email": "jonas@example.com", "role": "admin" });
        let name = json!({ "name": "Mia Example", "role": "user" });
        for (action, category, details, act) in [
            ("auth.login", Login, json!({}), " signed in"),
            (
                "auth.login",
                Login,
                json!({ "method": "recovery" }),
                " signed in with a recovery code",
            ),
            (
                "auth.reauth",
                Login,
                json!({ "method": "totp" }),
                " confirmed their sign-in",
            ),
            (
                "auth.login_failed",
                Login,
                json!({ "reason": "locked" }),
                ": Sign-in denied, account suspended",
            ),
            (
                "auth.login_failed",
                Login,
                json!({ "reason": "new" }),
                ": Sign-in denied",
            ),
            (
                "auth.sessions_revoke",
                Login,
                json!({ "sessions_ended": 1 }),
                " signed out 1 other device",
            ),
            (
                "auth.sessions_revoke",
                Login,
                json!({ "sessions_ended": 3 }),
                " signed out 3 other devices",
            ),
            (
                "auth.totp_add",
                Login,
                json!({ "name": "Pixel" }),
                " added the authenticator app “Pixel”",
            ),
            (
                "users.invite",
                Users,
                email.clone(),
                " invited jonas@example.com as an administrator",
            ),
            (
                "users.invite_role",
                Users,
                email,
                " gave jonas@example.com the administrator role",
            ),
            (
                "users.role",
                Users,
                name.clone(),
                " gave Mia Example the user role",
            ),
            ("users.lock", Users, name.clone(), " suspended Mia Example"),
            (
                "users.unlock",
                Users,
                name.clone(),
                " reactivated Mia Example",
            ),
            (
                "users.mfa_reset",
                Users,
                name,
                " reset multi-factor authentication for Mia Example",
            ),
            (
                "system_mail.update",
                Settings,
                json!({ "fields": [] }),
                " changed the email settings",
            ),
            (
                "audit.retention",
                Settings,
                json!({ "days": 365 }),
                " changed the audit log retention to 1 year",
            ),
            (
                "audit.retention",
                Settings,
                json!({ "days": 30 }),
                " changed the audit log retention to 30 days",
            ),
            (
                "auth.reset_admin",
                System,
                json!({ "user": "Anna Example" }),
                " reset access for Anna Example",
            ),
        ] {
            assert_eq!(shown_en(action, category, details).act, act, "{action}");
        }

        let mut setup = row("setup.complete", System, json!({}));
        setup.actor_name = None;
        let line = render(&setup, Language::English);
        assert_eq!(
            (line.who.as_str(), line.act.as_str()),
            ("System", " completed the setup")
        );
        let line = render(&setup, Language::German);
        assert_eq!(
            (line.who.as_str(), line.act.as_str()),
            ("System", " hat die Einrichtung abgeschlossen")
        );

        for (count, meta) in [(1, "1 entry"), (2, "2 entries")] {
            assert_eq!(
                shown_en("audit.export", System, json!({ "count": count })).meta,
                meta
            );
        }
    }

    #[test]
    fn missing_details_or_unknown_action_show_the_key() {
        let line = shown_en("users.lock", Category::Users, json!({}));
        assert_eq!((line.icon, line.act.as_str()), ("ti-users", ": users.lock"));
        let line = shown_en("apps.pair", Category::Apps, json!({}));
        assert_eq!(
            (line.icon, line.act.as_str(), line.warn),
            ("ti-apps", ": apps.pair", false)
        );
        assert_eq!(
            shown_en(
                "users.invite",
                Category::Users,
                json!({ "email": "a@example.com" })
            )
            .act,
            ": users.invite"
        );
    }

    #[test]
    fn only_sign_ins_show_the_short_ip() {
        let mut logout = row("auth.logout", Category::Login, json!({}));
        logout.ip = Some("198.51.100.56".into());
        assert_eq!(render(&logout, Language::English).meta, "198.51.x.x");

        let mut org = row("org.update", Category::Settings, json!({}));
        org.ip = Some("198.51.100.56".into());
        assert_eq!(render(&org, Language::English).meta, "");
    }

    #[test]
    fn head_categories_and_search() {
        assert_eq!(
            csv_head(Language::English),
            ["Time", "Category", "Person", "Action", "Details"]
        );
        assert_eq!(
            csv_head(Language::German),
            ["Zeitpunkt", "Bereich", "Person", "Aktion", "Details"]
        );
        assert_eq!(
            Category::ALL.map(|category| label(category, Language::English)),
            ["Sign-in", "Users", "Apps", "Settings", "System"]
        );
        assert_eq!(
            Category::ALL.map(|category| label(category, Language::German)),
            ["Anmeldung", "Benutzer", "Apps", "Einstellungen", "System"]
        );
        let mut login = row("auth.login", Category::Login, json!({}));
        login.ip = Some("198.51.100.56".into());
        assert_eq!(
            render(&login, Language::English).searchable(login.category, Language::English),
            "Anna Example signed in 198.51.x.x Sign-in"
        );
    }

    /// Every key has a text in both languages, with the same icons,
    /// highlights and details; German differs in wording only.
    #[test]
    fn every_action_has_a_text_in_both_languages() {
        for details in [all_details(), other_details()] {
            for (action, category) in ACTIONS {
                let row = row(action, category, details.clone());
                let (en, de) = (
                    render(&row, Language::English),
                    render(&row, Language::German),
                );
                assert!(!en.act.contains(action), "{action}: falls back to the key");
                assert_eq!(
                    (de.icon, de.warn, de.meta.is_empty(), &de.who),
                    (en.icon, en.warn, en.meta.is_empty(), &en.who),
                    "{action}: {details}"
                );
                assert_ne!(de.act, en.act, "{action}");
                for line in [&en, &de] {
                    assert!(
                        line.act.starts_with(": ")
                            || (line.act.starts_with(' ') && !line.act.starts_with("  ")),
                        "{action}: {}",
                        line.act
                    );
                    assert!(!line.act.contains(['{', '}']), "{action}: {}", line.act);
                }
                assert!(
                    en.act.starts_with(": ")
                        || en.act[1..].starts_with(|c: char| c.is_ascii_lowercase()),
                    "{action}: {}",
                    en.act
                );
            }
        }
        let fallback = row("users.lock", Category::Users, json!({}));
        assert_eq!(render(&fallback, Language::German).act, ": users.lock");
    }

    /// Every key the server writes (`Entry::new(…, "…")` and the actor's
    /// `entry("…")` in `src/`) has a text: the list above is complete.
    #[test]
    fn every_written_key_is_listed() {
        let mut written = std::collections::BTreeSet::new();
        let mut dirs = vec![std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        ))];
        while let Some(dir) = dirs.pop() {
            for file in std::fs::read_dir(dir).unwrap() {
                let path = file.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "rs") || path.ends_with("texts.rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                // Only the code before the tests.
                let code = source.split("#[cfg(test)]").next().unwrap_or_default();
                // The key is the first string in the call: after the
                // category, or right away with `actor.entry("…", …)`.
                for (start, index) in [("Entry::new(", 1), ("entry(\"", 0), ("audit_entry(", 1)] {
                    for part in code.split(start).skip(1) {
                        let call = part.split(')').next().unwrap_or_default();
                        if let Some(key) = call.split('"').nth(index) {
                            written.insert(key.to_owned());
                        }
                    }
                }
                // `let action = if … { "auth.email_change_request" } …`
                for part in code.split("let action = ").skip(1) {
                    let statement = part.split(';').next().unwrap_or_default();
                    for key in statement.split('"').skip(1).step_by(2) {
                        written.insert(key.to_owned());
                    }
                }
            }
        }
        let listed: std::collections::BTreeSet<String> =
            ACTIONS.iter().map(|(key, _)| (*key).to_owned()).collect();
        let missing: Vec<&String> = written.difference(&listed).collect();
        assert!(missing.is_empty(), "keys without a text: {missing:?}");
        assert!(written.len() >= 30, "found only {written:?}");
    }
}
