//! Table `stalwart_servers`: the registry of Stalwart servers
//! (Administration › Stalwart servers), plus the policy for custom servers
//! in `app_settings`. Input is validated by `api::stalwart`; this module only
//! reads and writes.
//!
//! Taken over from Roedy Mail (`crates/server/src/db/repo/servers.rs`)
//! without the pending DCR registrations: the hub registers its client for
//! a server that is already saved.

use std::time::Duration;

use anyhow::Context as _;
use rusqlite::{Connection, OptionalExtension as _, Row};
use serde::{Deserialize, Serialize};

use super::app_settings;
use crate::stalwart::probe::{Report, Status};

/// Purpose for the AAD of the encrypted client secret (`crypto`); the row
/// is the server's ID.
pub const SECRET_PURPOSE: &str = "stalwart_servers.oauth_client_secret";
/// Key of the policy in `app_settings`.
const POLICY: &str = "stalwart_policy";

/// A row of `stalwart_servers`. The client secret is stored only encrypted
/// and never leaves the hub; here only whether there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub id: String,
    pub label: String,
    /// Scheme, host and port, e.g. `https://mail.example.com`.
    pub base_url: String,
    pub domains: Vec<String>,
    pub client_id: Option<String>,
    pub has_secret: bool,
    pub reach: Status,
    pub cors: Status,
    pub oauth: Status,
    pub tested_at: Option<String>,
    pub created_at: String,
}

/// Validated fields for [`insert`] and [`update`].
pub struct Fields<'a> {
    pub label: &'a str,
    pub base_url: &'a str,
    pub domains: &'a [String],
    pub client_id: Option<&'a str>,
}

/// What happens to the encrypted client secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretChange {
    Keep,
    Clear,
    /// New value from `crypto::SecretKey::seal`.
    Set(Vec<u8>),
}

const COLUMNS: &str = "id, label, base_url, domains_json, oauth_client_id, \
     oauth_client_secret_enc IS NOT NULL, reach, cors, oauth, tested_at, created_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<(Server, String)> {
    let status = |i| {
        row.get::<_, String>(i)
            .map(|value| Status::parse(&value).unwrap_or(Status::Unk))
    };
    Ok((
        Server {
            id: row.get(0)?,
            label: row.get(1)?,
            base_url: row.get(2)?,
            domains: Vec::new(),
            client_id: row.get(4)?,
            has_secret: row.get(5)?,
            reach: status(6)?,
            cors: status(7)?,
            oauth: status(8)?,
            tested_at: row.get(9)?,
            created_at: row.get(10)?,
        },
        row.get(3)?,
    ))
}

fn with_domains((mut server, json): (Server, String)) -> anyhow::Result<Server> {
    server.domains = serde_json::from_str(&json)
        .with_context(|| format!("stalwart_servers.domains_json of {} is invalid", server.id))?;
    Ok(server)
}

/// All servers in the order they were created (ULIDs).
pub fn list(conn: &Connection) -> anyhow::Result<Vec<Server>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM stalwart_servers ORDER BY id"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    rows.map(|row| with_domains(row?)).collect()
}

pub fn get(conn: &Connection, id: &str) -> anyhow::Result<Option<Server>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM stalwart_servers WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()?
    .map(with_domains)
    .transpose()
}

/// Server with the origin `base_url`.
pub fn find_by_base_url(conn: &Connection, base_url: &str) -> anyhow::Result<Option<Server>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM stalwart_servers WHERE base_url = ?1"),
        [base_url],
        from_row,
    )
    .optional()?
    .map(with_domains)
    .transpose()
}

/// Encrypted client secret of the server (code exchange, refresh and
/// revocation), only while the server still has the address and client the
/// caller read with it: an administrator may have changed both since, and a
/// secret must never go to an address or client it was not entered for. One
/// query, so the check cannot race with [`update`].
pub fn client_secret(
    conn: &Connection,
    id: &str,
    base_url: &str,
    client_id: &str,
) -> rusqlite::Result<Option<Vec<u8>>> {
    conn.query_row(
        "SELECT oauth_client_secret_enc FROM stalwart_servers \
         WHERE id = ?1 AND base_url = ?2 AND oauth_client_id = ?3",
        (id, base_url, client_id),
        |row| row.get(0),
    )
    .optional()
    .map(Option::flatten)
}

/// Creates a server with the ID `id` (`db::new_id`; encrypting the secret
/// needs it beforehand).
pub fn insert(
    conn: &Connection,
    id: &str,
    fields: &Fields<'_>,
    secret: Option<&[u8]>,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO stalwart_servers (id, label, base_url, domains_json, oauth_client_id, \
         oauth_client_secret_enc) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (
            id,
            fields.label,
            fields.base_url,
            serde_json::to_string(fields.domains)?,
            fields.client_id,
            secret,
        ),
    )
    .context("Stalwart server not created")?;
    Ok(())
}

/// Writes all fields; with `reset` the last diagnosis no longer applies
/// (address or client changed). `false` if the server does not exist.
pub fn update(
    conn: &Connection,
    id: &str,
    fields: &Fields<'_>,
    secret: &SecretChange,
    reset: bool,
) -> anyhow::Result<bool> {
    let (mode, value) = match secret {
        SecretChange::Keep => (0, None),
        SecretChange::Clear => (1, None),
        SecretChange::Set(value) => (2, Some(value.as_slice())),
    };
    let changed = conn
        .execute(
            "UPDATE stalwart_servers SET label = ?2, base_url = ?3, domains_json = ?4, \
             oauth_client_id = ?5, \
             oauth_client_secret_enc = CASE ?6 WHEN 0 THEN oauth_client_secret_enc \
                                               WHEN 1 THEN NULL ELSE ?7 END, \
             reach = CASE WHEN ?8 THEN 'unk' ELSE reach END, \
             cors = CASE WHEN ?8 THEN 'unk' ELSE cors END, \
             oauth = CASE WHEN ?8 THEN 'unk' ELSE oauth END, \
             tested_at = CASE WHEN ?8 THEN NULL ELSE tested_at END \
             WHERE id = ?1",
            (
                id,
                fields.label,
                fields.base_url,
                serde_json::to_string(fields.domains)?,
                fields.client_id,
                mode,
                value,
                reset,
            ),
        )
        .context("Stalwart server not updated")?;
    Ok(changed == 1)
}

/// `false` if the server does not exist.
pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute("DELETE FROM stalwart_servers WHERE id = ?1", [id])? == 1)
}

/// Stores the result of a diagnosis – only if address and client ID are
/// still the ones tested. If someone changed the server in the meantime, the
/// result would apply to the old state; then `false`.
pub fn record_test(
    conn: &Connection,
    id: &str,
    base_url: &str,
    client_id: Option<&str>,
    report: &Report,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE stalwart_servers SET reach = ?4, cors = ?5, oauth = ?6, \
         tested_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE id = ?1 AND base_url = ?2 AND oauth_client_id IS ?3",
        (
            id,
            base_url,
            client_id,
            report.reach.as_str(),
            report.cors.as_str(),
            report.oauth.as_str(),
        ),
    )?;
    Ok(changed == 1)
}

/// Servers whose last test is older than `age` or that were never tested
/// (job `server_probe`).
pub fn due_for_test(conn: &Connection, age: Duration) -> anyhow::Result<Vec<Server>> {
    let modifier = format!("-{} seconds", age.as_secs());
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM stalwart_servers \
         WHERE tested_at IS NULL OR tested_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1) \
         ORDER BY id"
    ))?;
    let rows = stmt
        .query_map([modifier], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(with_domains).collect()
}

/// Origins of all servers (the apps' `connect-src`).
pub fn base_urls(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT base_url FROM stalwart_servers ORDER BY base_url")?;
    stmt.query_map([], |row| row.get(0))?.collect()
}

/// The policy as stored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Policy {
    allow_custom_servers: bool,
}

/// Policy "Allow custom servers" (off by default).
pub fn allow_custom_servers(conn: &Connection) -> anyhow::Result<bool> {
    Ok(app_settings::get::<Policy>(conn, POLICY)?
        .unwrap_or_default()
        .allow_custom_servers)
}

/// `true` if the value changed.
pub fn set_allow_custom_servers(conn: &Connection, allow: bool) -> anyhow::Result<bool> {
    if allow_custom_servers(conn)? == allow {
        return Ok(false);
    }
    app_settings::set(
        conn,
        POLICY,
        &Policy {
            allow_custom_servers: allow,
        },
    )?;
    Ok(true)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::repo::users::tests::conn;

    pub(crate) fn add(conn: &Connection, id: &str, base_url: &str) {
        insert(
            conn,
            id,
            &Fields {
                label: "Mail",
                base_url,
                domains: &["example.com".to_owned()],
                client_id: Some("roedy-hub"),
            },
            None,
        )
        .unwrap();
    }

    #[test]
    fn servers_by_id_and_address() {
        let conn = conn();
        add(&conn, "s1", "https://mail.s1.test");
        let server = get(&conn, "s1").unwrap().unwrap();
        assert_eq!(server.domains, ["example.com"]);
        assert_eq!(server.reach, Status::Unk);
        assert!(!server.has_secret);
        assert_eq!(
            find_by_base_url(&conn, "https://mail.s1.test").unwrap(),
            Some(server)
        );
        assert_eq!(base_urls(&conn).unwrap(), ["https://mail.s1.test"]);
        // A secret only while address and client stay.
        let fields = Fields {
            label: "Mail",
            base_url: "https://mail.s1.test",
            domains: &[],
            client_id: Some("roedy-hub"),
        };
        assert!(update(&conn, "s1", &fields, &SecretChange::Set(vec![7]), false).unwrap());
        assert_eq!(
            client_secret(&conn, "s1", "https://mail.s1.test", "roedy-hub").unwrap(),
            Some(vec![7])
        );
        assert_eq!(
            client_secret(&conn, "s1", "https://mail.s2.test", "roedy-hub").unwrap(),
            None
        );
        assert_eq!(
            client_secret(&conn, "s1", "https://mail.s1.test", "other").unwrap(),
            None
        );
        assert!(update(&conn, "s1", &fields, &SecretChange::Clear, true).unwrap());
        assert!(!get(&conn, "s1").unwrap().unwrap().has_secret);
        assert!(delete(&conn, "s1").unwrap());
        assert!(!delete(&conn, "s1").unwrap());
    }

    #[test]
    fn tests_only_count_for_the_state_tested() {
        let conn = conn();
        add(&conn, "s1", "https://mail.s1.test");
        let report = Report {
            reach: Status::Ok,
            ..Report::default()
        };
        assert!(!record_test(&conn, "s1", "https://mail.s1.test", Some("other"), &report).unwrap());
        assert_eq!(
            due_for_test(&conn, Duration::from_secs(60)).unwrap().len(),
            1
        );
        assert!(
            record_test(
                &conn,
                "s1",
                "https://mail.s1.test",
                Some("roedy-hub"),
                &report
            )
            .unwrap()
        );
        assert_eq!(get(&conn, "s1").unwrap().unwrap().reach, Status::Ok);
        assert!(
            due_for_test(&conn, Duration::from_secs(60))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn custom_servers_are_off_by_default() {
        let conn = conn();
        assert!(!allow_custom_servers(&conn).unwrap());
        assert!(!set_allow_custom_servers(&conn, false).unwrap());
        assert!(set_allow_custom_servers(&conn, true).unwrap());
        assert!(allow_custom_servers(&conn).unwrap());
    }
}
