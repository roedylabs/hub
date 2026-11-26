//! Instance-wide details for everyone: organization and branding
//! (Administration › Organization and Branding).
//!
//! They live in `app_settings` (keys [`ORG`] and [`BRANDING`]) and in memory
//! ([`Instance`], loaded at startup, set after every change while still on
//! the database thread). `GET session`, `GET public/config`, the issuer of
//! authenticator apps and `AppState::brand` (mails) read from there – one
//! source without a database query per request.
//! Changed by: `api::organization` and `api::branding`.
//!
//! Taken over from Roedy Mail (`crates/server/src/instance.rs`) without the
//! banner, the update hint, the browser cache version and the logo (which
//! needs `resvg` and fonts to render it for mails); the hub's name is the
//! organization's name.

use std::sync::{PoisonError, RwLock};

use rusqlite::Connection;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{db::repo::app_settings, mailer::templates::APP_NAME};

/// Key of the organization in `app_settings`.
pub const ORG: &str = "org";
/// Key of the branding in `app_settings`.
pub const BRANDING: &str = "branding";

/// Organization details; `None` means "not specified" (an empty field on the
/// page).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Org {
    /// Also the hub's name (`GET session` › `hub.name`, mails, authenticator
    /// apps); without it "Roedy Hub".
    pub name: Option<String>,
    pub website: Option<String>,
    pub help: Option<String>,
    /// Support email.
    pub email: Option<String>,
    pub imprint: Option<String>,
    pub privacy: Option<String>,
    pub terms: Option<String>,
    /// Repository address (source code, AGPL).
    pub source: Option<String>,
}

impl Org {
    /// The fields in the order of the page, for validation and the audit log.
    pub fn fields_mut(&mut self) -> [(&'static str, &mut Option<String>); 8] {
        [
            ("name", &mut self.name),
            ("website", &mut self.website),
            ("help", &mut self.help),
            ("email", &mut self.email),
            ("imprint", &mut self.imprint),
            ("privacy", &mut self.privacy),
            ("terms", &mut self.terms),
            ("source", &mut self.source),
        ]
    }

    /// The hub's name: the organization's or "Roedy Hub".
    pub fn hub_name(&self) -> &str {
        self.name.as_deref().unwrap_or(APP_NAME)
    }
}

/// Branding: the accent colour (one of `mailer::templates::ACCENTS`) and
/// whether users may choose their own (`userAccent`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Branding {
    pub color: String,
    pub user_accent: bool,
}

impl Default for Branding {
    /// Apricot; users may choose their own accent.
    fn default() -> Self {
        Self {
            color: "apricot".into(),
            user_accent: true,
        }
    }
}

/// Reads `key`; `T` itself tolerates missing and unknown fields. If a value
/// does not fit (say a text instead of a number), `fallback` applies with a
/// warning – otherwise the server would not start, and `PUT` could never
/// replace the entry.
fn load<T: DeserializeOwned>(
    conn: &Connection,
    key: &str,
    fallback: impl FnOnce(&Value) -> T,
) -> anyhow::Result<Option<T>> {
    let Some(value) = app_settings::get::<Value>(conn, key)? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_value(value.clone()).unwrap_or_else(
        |error| {
            tracing::warn!("app_settings.{key} unreadable, using default: {error}");
            fallback(&value)
        },
    )))
}

/// Organization from the database; without a (readable) entry nothing is
/// specified.
pub fn load_org(conn: &Connection) -> anyhow::Result<Org> {
    Ok(load(conn, ORG, |_| Org::default())?.unwrap_or_default())
}

/// Branding from the database; without a (readable) entry the default.
pub fn load_branding(conn: &Connection) -> anyhow::Result<Branding> {
    Ok(load(conn, BRANDING, |_| Branding::default())?.unwrap_or_default())
}

/// The in-memory state.
#[derive(Debug, Default)]
pub struct Instance {
    org: RwLock<Org>,
    branding: RwLock<Branding>,
}

impl Instance {
    /// Reads everything from the database.
    pub fn load(&self, conn: &Connection) -> anyhow::Result<()> {
        self.set_org(load_org(conn)?);
        self.set_branding(load_branding(conn)?);
        Ok(())
    }

    pub fn org(&self) -> Org {
        self.org
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_org(&self, org: Org) {
        *self.org.write().unwrap_or_else(PoisonError::into_inner) = org;
    }

    pub fn branding(&self) -> Branding {
        self.branding
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_branding(&self, branding: Branding) {
        *self
            .branding
            .write()
            .unwrap_or_else(PoisonError::into_inner) = branding;
    }

    /// The hub's name ([`Org::hub_name`]).
    pub fn name(&self) -> String {
        self.org
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .hub_name()
            .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;

    fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    #[test]
    fn defaults_without_entries() {
        let conn = conn();
        assert_eq!(load_org(&conn).unwrap(), Org::default());
        assert_eq!(
            serde_json::to_value(load_branding(&conn).unwrap()).unwrap(),
            serde_json::json!({ "color": "apricot", "userAccent": true })
        );
        let instance = Instance::default();
        instance.load(&conn).unwrap();
        assert_eq!(instance.name(), "Roedy Hub");
    }

    #[test]
    fn reads_tolerantly() {
        let conn = conn();
        // Missing and unknown fields from another program version.
        conn.execute(
            "INSERT INTO app_settings (key, json) VALUES \
             ('org', '{\"name\":\"Example Inc.\",\"fax\":\"0\"}'), \
             ('branding', '{\"color\":\"sage\",\"kind\":\"image\"}')",
            [],
        )
        .unwrap();
        let instance = Instance::default();
        instance.load(&conn).unwrap();
        assert_eq!(instance.name(), "Example Inc.");
        assert_eq!(instance.org().email, None);
        assert_eq!(
            instance.branding(),
            Branding {
                color: "sage".into(),
                user_accent: true
            }
        );
    }

    #[test]
    fn unreadable_entries_fall_back_instead_of_failing() {
        let conn = conn();
        conn.execute(
            "INSERT INTO app_settings (key, json) VALUES \
             ('org', '{\"name\":7}'), ('branding', '{\"userAccent\":\"yes\"}')",
            [],
        )
        .unwrap();
        assert_eq!(load_org(&conn).unwrap(), Org::default());
        assert_eq!(load_branding(&conn).unwrap(), Branding::default());
    }
}
