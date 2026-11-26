//! Table `audit_log`: admin and security actions. Each task records its own
//! actions, in the same transaction as the change; reading, the texts and
//! the retention follow with Administration › Audit log.
//!
//! Taken over from Roedy Mail (`crates/server/src/db/repo/audit_log.rs`)
//! without the place (no GeoIP) and with the category `apps`.

use std::net::IpAddr;

use rusqlite::Connection;
use serde_json::Value;

use crate::{auth::session::AuthUser, db::repo::users::Profile};

/// Categories of the audit log (the filter of the audit log page).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Sign-ins and the own account
    Login,
    /// Users
    Users,
    /// Apps
    Apps,
    /// Settings
    Settings,
    /// System
    System,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Users => "users",
            Self::Apps => "apps",
            Self::Settings => "settings",
            Self::System => "system",
        }
    }
}

/// An entry. `action` is a stable key (`auth.login`, `users.lock` …) that the
/// audit log page turns into text.
#[derive(Debug, Clone)]
pub struct Entry {
    pub category: Category,
    pub action: &'static str,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub target: Option<String>,
    pub ip: Option<IpAddr>,
    pub details: Option<Value>,
}

impl Entry {
    pub fn new(category: Category, action: &'static str) -> Self {
        Self {
            category,
            action,
            actor_id: None,
            actor_name: None,
            target: None,
            ip: None,
            details: None,
        }
    }

    pub fn actor(mut self, user: &AuthUser) -> Self {
        self.actor_id = Some(user.id.clone());
        self.actor_name = Some(user.display_name.clone());
        self
    }

    /// Actor without a session (failed sign-in) or with the state from the
    /// database (deleting the account).
    pub fn actor_profile(mut self, user: &Profile) -> Self {
        self.actor_id = Some(user.id.clone());
        self.actor_name = Some(user.display_name.clone());
        self
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn ip(mut self, ip: IpAddr) -> Self {
        self.ip = Some(ip);
        self
    }

    /// Further details as JSON – never passwords, tokens or codes.
    pub fn details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

pub fn insert(conn: &Connection, entry: &Entry) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO audit_log (category, action, actor_id, actor_name, target, ip, details) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            entry.category.as_str(),
            entry.action,
            &entry.actor_id,
            &entry.actor_name,
            &entry.target,
            entry.ip.map(|ip| ip.to_string()),
            entry.details.as_ref().map(Value::to_string),
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::db::repo::users::{
        Role,
        tests::{add, conn},
    };

    #[test]
    fn stores_entries_and_keeps_the_name_of_deleted_users() {
        let conn = conn();
        let id = add(&conn, "max", "max@example.com", Role::User);
        let profile = crate::db::repo::users::credentials_by_id(&conn, &id)
            .unwrap()
            .unwrap()
            .user;
        for category in [
            Category::Login,
            Category::Users,
            Category::Apps,
            Category::Settings,
            Category::System,
        ] {
            insert(
                &conn,
                &Entry::new(category, "test.action")
                    .actor_profile(&profile)
                    .target(&id)
                    .ip("198.51.100.7".parse().unwrap())
                    .details(json!({ "fields": ["display_name"] })),
            )
            .unwrap();
        }
        conn.execute("DELETE FROM users WHERE id = ?1", [&id])
            .unwrap();
        let rows: Vec<(String, Option<String>, String, String, String)> = conn
            .prepare(
                "SELECT category, actor_id, actor_name, ip, details FROM audit_log ORDER BY id",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let categories: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(categories, ["login", "users", "apps", "settings", "system"]);
        let first = &rows[0];
        assert_eq!(first.1, None, "actor_id is cleared with the user");
        assert_eq!(first.2, "Max Example");
        assert_eq!(first.3, "198.51.100.7");
        assert_eq!(first.4, r#"{"fields":["display_name"]}"#);
    }
}
