//! Tables `apps` and `pairing_codes`: the apps behind the gateway and the
//! open pairing code of each (docs/app-contract.md › Pairing).

use rusqlite::{Connection, OptionalExtension as _, types::Type};

use roedy_hub_contract::Manifest;

use crate::db;

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// A pairing code is valid this long (contract § 9).
pub const CODE_VALID_MINUTES: u32 = 15;

/// Whether the app has paired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Added, waiting for the first pairing.
    Pending,
    Paired,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Paired => "paired",
        }
    }
}

/// Access of new users (contract § 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessDefault {
    /// "Everyone": new users get access with the default role.
    All,
    /// "Selected only": new users get no access.
    Selected,
}

impl AccessDefault {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Selected => "selected",
        }
    }
}

/// Result of the last health check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Online,
    Offline,
}

impl Health {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "online" => Some(Self::Online),
            "offline" => Some(Self::Offline),
            _ => None,
        }
    }
}

/// An app as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub slug: String,
    /// Origin at which the hub reaches the app, e.g. `http://roedy-mail:8080`.
    pub internal_url: String,
    pub status: Status,
    /// The last manifest (with the sanitized icon); `None` while pending.
    pub manifest: Option<Manifest>,
    pub access_default: AccessDefault,
    pub position: i64,
    pub health: Option<Health>,
    pub health_at: Option<String>,
    pub paired_at: Option<String>,
    pub created_at: String,
    /// Expiry of the open, unexpired pairing code.
    pub pairing_expires_at: Option<String>,
}

const SELECT: &str = "SELECT a.id, a.slug, a.internal_url, a.state, a.manifest_json, \
                             a.access_default, a.position, a.health, a.health_at, a.paired_at, \
                             a.created_at, \
                             (SELECT p.expires_at FROM pairing_codes p \
                              WHERE p.app_id = a.id \
                                AND p.expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
                      FROM apps a";

fn app(row: &rusqlite::Row<'_>) -> rusqlite::Result<App> {
    let state: String = row.get(3)?;
    let manifest: Option<String> = row.get(4)?;
    let manifest = manifest
        .map(|json| serde_json::from_str::<Manifest>(&json))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
        })?;
    let access: String = row.get(5)?;
    let health: Option<String> = row.get(7)?;
    Ok(App {
        id: row.get(0)?,
        slug: row.get(1)?,
        internal_url: row.get(2)?,
        status: if state == "paired" {
            Status::Paired
        } else {
            Status::Pending
        },
        manifest,
        access_default: if access == "selected" {
            AccessDefault::Selected
        } else {
            AccessDefault::All
        },
        position: row.get(6)?,
        health: health.as_deref().and_then(Health::parse),
        health_at: row.get(8)?,
        paired_at: row.get(9)?,
        created_at: row.get(10)?,
        pairing_expires_at: row.get(11)?,
    })
}

/// Adds a pending app at the end of the order and returns its ID. A taken
/// slug fails on the uniqueness constraint; check [`slug_taken`] first.
pub fn insert(conn: &Connection, slug: &str, internal_url: &str) -> anyhow::Result<String> {
    let id = db::new_id()?;
    conn.execute(
        "INSERT INTO apps (id, slug, internal_url, position) \
         VALUES (?1, ?2, ?3, (SELECT coalesce(max(position), 0) + 1 FROM apps))",
        (&id, slug, internal_url),
    )?;
    Ok(id)
}

pub fn slug_taken(conn: &Connection, slug: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM apps WHERE slug = ?1)",
        [slug],
        |row| row.get(0),
    )
}

/// All apps in their order.
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<App>> {
    conn.prepare(&format!("{SELECT} ORDER BY a.position, a.created_at"))?
        .query_map([], app)?
        .collect()
}

pub fn by_slug(conn: &Connection, slug: &str) -> rusqlite::Result<Option<App>> {
    conn.query_row(&format!("{SELECT} WHERE a.slug = ?1"), [slug], app)
        .optional()
}

pub fn by_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<App>> {
    conn.query_row(&format!("{SELECT} WHERE a.id = ?1"), [id], app)
        .optional()
}

/// The paired app whose secret has this SHA-256.
pub fn by_secret_hash(conn: &Connection, hash: &[u8; 32]) -> rusqlite::Result<Option<App>> {
    conn.query_row(
        &format!("{SELECT} WHERE a.state = 'paired' AND a.secret_hash = ?1"),
        [hash.as_slice()],
        app,
    )
    .optional()
}

/// Removes the app with its pairing code and access entries; its secret
/// stops working.
pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute("DELETE FROM apps WHERE id = ?1", [id])? == 1)
}

/// The app paired (again): manifest and the new secret's hash; the previous
/// secret stops working.
pub fn pair(
    conn: &Connection,
    id: &str,
    manifest: &Manifest,
    secret_hash: &[u8; 32],
) -> anyhow::Result<()> {
    conn.execute(
        &format!(
            "UPDATE apps SET state = 'paired', manifest_json = ?2, manifest_id = ?3, \
                             version = ?4, secret_hash = ?5, paired_at = {NOW} \
             WHERE id = ?1"
        ),
        (
            id,
            serde_json::to_string(manifest)?,
            &manifest.id,
            &manifest.version,
            secret_hash.as_slice(),
        ),
    )?;
    Ok(())
}

/// A new manifest of a paired app.
pub fn set_manifest(conn: &Connection, id: &str, manifest: &Manifest) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE apps SET manifest_json = ?2, manifest_id = ?3, version = ?4 \
         WHERE id = ?1 AND state = 'paired'",
        (
            id,
            serde_json::to_string(manifest)?,
            &manifest.id,
            &manifest.version,
        ),
    )?;
    Ok(())
}

/// Stores a health check; `false` if the app is gone or no longer paired.
pub fn set_health(conn: &Connection, id: &str, health: Health) -> rusqlite::Result<bool> {
    let n = conn.execute(
        &format!(
            "UPDATE apps SET health = ?2, health_at = {NOW} WHERE id = ?1 AND state = 'paired'"
        ),
        (id, health.as_str()),
    )?;
    Ok(n == 1)
}

/// A new pairing code for the app, replacing the open one; returns its
/// expiry.
pub fn set_pairing_code(
    conn: &Connection,
    app_id: &str,
    code_hash: &[u8; 32],
    created_by: Option<&str>,
) -> rusqlite::Result<String> {
    conn.query_row(
        &format!(
            "INSERT INTO pairing_codes (app_id, code_hash, expires_at, created_by) \
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', \
                                      '+{CODE_VALID_MINUTES} minutes'), ?3) \
             ON CONFLICT (app_id) DO UPDATE SET code_hash = excluded.code_hash, \
                 expires_at = excluded.expires_at, created_by = excluded.created_by, \
                 created_at = {NOW} \
             RETURNING expires_at"
        ),
        (app_id, code_hash.as_slice(), created_by),
        |row| row.get(0),
    )
}

/// The app of an unexpired code.
pub fn app_for_code(conn: &Connection, code_hash: &[u8; 32]) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        &format!("SELECT app_id FROM pairing_codes WHERE code_hash = ?1 AND expires_at > {NOW}"),
        [code_hash.as_slice()],
        |row| row.get(0),
    )
    .optional()
}

/// Uses up an unexpired code; returns its app.
pub fn take_code(conn: &Connection, code_hash: &[u8; 32]) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        &format!(
            "DELETE FROM pairing_codes WHERE code_hash = ?1 AND expires_at > {NOW} \
             RETURNING app_id"
        ),
        [code_hash.as_slice()],
        |row| row.get(0),
    )
    .optional()
}

/// Removes expired codes (`session_cleanup`).
pub fn delete_expired_codes(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        &format!("DELETE FROM pairing_codes WHERE expires_at <= {NOW}"),
        [],
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use super::*;
    use crate::db::repo::users::tests::conn;

    pub(crate) fn manifest() -> Manifest {
        Manifest::parse(&json!({
            "contract": 1,
            "id": "roedy.example",
            "version": "1.0.0",
            "name": { "en": "Example", "de": "Beispiel" },
            "description": { "en": "", "de": "" },
            "icon": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"/>",
            "roles": [
                { "id": "user", "name": { "en": "User", "de": "Benutzer" },
                  "description": { "en": "", "de": "" }, "default": true },
                { "id": "admin", "name": { "en": "Admin", "de": "Admin" },
                  "description": { "en": "", "de": "" }, "admin": true }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn pending_then_paired() {
        let conn = conn();
        let mail = insert(&conn, "mail", "http://mail.test:8080").unwrap();
        let notes = insert(&conn, "notes", "http://notes.test:8080").unwrap();
        assert!(slug_taken(&conn, "mail").unwrap());
        assert!(insert(&conn, "mail", "http://x.test").is_err(), "unique");
        let apps = list(&conn).unwrap();
        assert_eq!(
            apps.iter()
                .map(|a| (a.slug.as_str(), a.position, a.status))
                .collect::<Vec<_>>(),
            [("mail", 1, Status::Pending), ("notes", 2, Status::Pending)]
        );
        assert_eq!(apps[0].access_default, AccessDefault::All);
        assert_eq!(apps[0].manifest, None);
        assert!(
            !set_health(&conn, &mail, Health::Online).unwrap(),
            "pending"
        );

        pair(&conn, &mail, &manifest(), &[7; 32]).unwrap();
        let paired = by_secret_hash(&conn, &[7; 32]).unwrap().unwrap();
        assert_eq!(paired.id, mail);
        assert_eq!(paired.status, Status::Paired);
        assert_eq!(paired.manifest, Some(manifest()));
        assert!(paired.paired_at.is_some());
        assert!(by_secret_hash(&conn, &[8; 32]).unwrap().is_none());
        assert!(set_health(&conn, &mail, Health::Offline).unwrap());
        let row = by_slug(&conn, "mail").unwrap().unwrap();
        assert_eq!(row.health, Some(Health::Offline));
        assert!(row.health_at.is_some());

        // Pairing again replaces the secret.
        pair(&conn, &mail, &manifest(), &[8; 32]).unwrap();
        assert!(by_secret_hash(&conn, &[7; 32]).unwrap().is_none());
        assert!(by_secret_hash(&conn, &[8; 32]).unwrap().is_some());

        assert!(delete(&conn, &notes).unwrap());
        assert!(!delete(&conn, &notes).unwrap());
        assert!(by_id(&conn, &notes).unwrap().is_none());
    }

    #[test]
    fn one_open_code_per_app() {
        let conn = conn();
        let app = insert(&conn, "mail", "http://mail.test:8080").unwrap();
        let expires = set_pairing_code(&conn, &app, &[1; 32], None).unwrap();
        assert_eq!(
            by_id(&conn, &app).unwrap().unwrap().pairing_expires_at,
            Some(expires)
        );
        assert_eq!(app_for_code(&conn, &[1; 32]).unwrap(), Some(app.clone()));
        // A new code replaces the old one.
        set_pairing_code(&conn, &app, &[2; 32], None).unwrap();
        assert_eq!(app_for_code(&conn, &[1; 32]).unwrap(), None);
        assert_eq!(take_code(&conn, &[2; 32]).unwrap(), Some(app.clone()));
        assert_eq!(take_code(&conn, &[2; 32]).unwrap(), None, "single use");
        // Expired codes do not count and are cleaned up.
        set_pairing_code(&conn, &app, &[3; 32], None).unwrap();
        conn.execute(
            "UPDATE pairing_codes SET expires_at = '2000-01-01T00:00:00.000Z'",
            [],
        )
        .unwrap();
        assert_eq!(app_for_code(&conn, &[3; 32]).unwrap(), None);
        assert_eq!(take_code(&conn, &[3; 32]).unwrap(), None);
        assert_eq!(
            by_id(&conn, &app).unwrap().unwrap().pairing_expires_at,
            None
        );
        assert_eq!(delete_expired_codes(&conn).unwrap(), 1);
        // Removing the app removes its code.
        set_pairing_code(&conn, &app, &[4; 32], None).unwrap();
        delete(&conn, &app).unwrap();
        let codes: i64 = conn
            .query_row("SELECT count(*) FROM pairing_codes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(codes, 0);
    }

    #[test]
    fn schema_keeps_paired_apps_complete() {
        let conn = conn();
        let app = insert(&conn, "mail", "http://mail.test:8080").unwrap();
        for sql in [
            "UPDATE apps SET state = 'paired' WHERE id = ?1",
            "UPDATE apps SET secret_hash = zeroblob(31) WHERE id = ?1",
            "UPDATE apps SET health = 'unknown' WHERE id = ?1",
            "UPDATE apps SET access_default = 'none' WHERE id = ?1",
            "UPDATE apps SET manifest_id = 'x' WHERE id = ?1",
            "UPDATE apps SET secret_hash = zeroblob(32) WHERE id = ?1",
            "UPDATE apps SET paired_at = '2026-01-01T00:00:00.000Z' WHERE id = ?1",
        ] {
            assert!(conn.execute(sql, [&app]).is_err(), "{sql}");
        }
        // Paired: no part may go missing.
        pair(&conn, &app, &manifest(), &[1; 32]).unwrap();
        for sql in [
            "UPDATE apps SET version = NULL WHERE id = ?1",
            "UPDATE apps SET manifest_id = NULL WHERE id = ?1",
            "UPDATE apps SET secret_hash = NULL WHERE id = ?1",
            "UPDATE apps SET paired_at = NULL WHERE id = ?1",
            "UPDATE apps SET state = 'pending' WHERE id = ?1",
        ] {
            assert!(conn.execute(sql, [&app]).is_err(), "{sql}");
        }
    }
}
