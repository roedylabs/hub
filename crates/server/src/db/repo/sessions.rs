//! Table `sessions`: hub sessions. Time arithmetic happens in SQL so that
//! expiry and throttling match the stored UTC timestamps.
//!
//! Taken over from Roedy Mail (`crates/server/src/db/repo/sessions.rs`)
//! without place, installed app, backups and maintenance mode; with
//! `auth_time` for the apps (docs/app-contract.md › Identity).

use rusqlite::{Connection, OptionalExtension as _};

use crate::{
    auth::session::{AuthUser, Client, IDLE},
    db::{self, repo::users::Role},
};

/// Write `last_seen_at` at most once per minute, not on every request
/// ("Last active" shows minutes anyway).
const TOUCH_AFTER: &str = "-60 seconds";
/// The user agent comes from the client; device detection needs no more.
pub const MAX_USER_AGENT: usize = 512;
/// At most this many sessions per user (a new one ends the least recently
/// used ones beyond that, see [`trim`]).
pub const MAX_PER_USER: u32 = 20;

fn idle_modifier() -> String {
    format!("+{} seconds", IDLE.as_secs())
}

/// Creates a session; returns its ID and `auth_time` in Unix seconds.
pub fn insert(
    conn: &Connection,
    user_id: &str,
    token_hash: &[u8; 32],
    client: &Client<'_>,
    persistent: bool,
) -> anyhow::Result<(String, i64)> {
    let id = db::new_id()?;
    let auth_time = conn.query_row(
        "INSERT INTO sessions (id, user_id, token_hash, user_agent, ip, persistent, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?7)) \
         RETURNING CAST(strftime('%s', auth_time) AS INTEGER)",
        (
            &id,
            user_id,
            token_hash.as_slice(),
            client.user_agent.map(|ua| truncate(ua, MAX_USER_AGENT)),
            client.ip.map(|ip| ip.to_string()),
            persistent,
            idle_modifier(),
        ),
        |row| row.get(0),
    )?;
    Ok((id, auth_time))
}

/// Session for the token hash if it has not expired and its user is active.
/// Extends it (and `users.last_seen_at`) once the last activity is more than
/// a minute ago; then `touched` is set and the cookie is renewed. If only the
/// extension fails (say because another process is writing), the request
/// stays authenticated – it works the next time.
pub fn authenticate(
    conn: &Connection,
    token_hash: &[u8; 32],
) -> anyhow::Result<Option<Authenticated>> {
    let found = conn
        .query_row(
            "SELECT s.id, s.last_seen_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?2), \
                    u.id, u.username, u.email, u.display_name, u.role, s.persistent, \
                    CAST(strftime('%s', s.auth_time) AS INTEGER) \
             FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = ?1 \
               AND s.expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
               AND u.status = 'active'",
            (token_hash.as_slice(), TOUCH_AFTER),
            |row| {
                let role: String = row.get(6)?;
                Ok((
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(7)?,
                    AuthUser {
                        session_id: row.get(0)?,
                        id: row.get(2)?,
                        username: row.get(3)?,
                        email: row.get(4)?,
                        display_name: row.get(5)?,
                        role: Role::parse(&role).unwrap_or(Role::User),
                        auth_time: row.get(8)?,
                    },
                ))
            },
        )
        .optional()?;
    let Some((stale, persistent, user)) = found else {
        return Ok(None);
    };
    let touched = stale
        && touch(conn, &user)
            .inspect_err(|error| tracing::warn!(%error, "session not extended"))
            .is_ok();
    Ok(Some(Authenticated {
        user,
        touched,
        persistent,
    }))
}

pub struct Authenticated {
    pub user: AuthUser,
    pub touched: bool,
    /// "Stay signed in": the renewed cookie keeps its kind.
    pub persistent: bool,
}

/// Session and user together, so that not just one of them takes effect.
fn touch(conn: &Connection, user: &AuthUser) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE sessions SET last_seen_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), \
                             expires_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?2) \
         WHERE id = ?1",
        (&user.session_id, idle_modifier()),
    )?;
    tx.execute(
        "UPDATE users SET last_seen_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
        [&user.id],
    )?;
    tx.commit()
}

/// A fresh sign-in in an existing session (`/login?reauth=1`): sets
/// `auth_time` to now and returns it in Unix seconds; `None` if the session
/// of the user no longer exists. The session keeps its ID, so the apps keep
/// what they bound to it.
pub fn reauthenticate(
    conn: &Connection,
    user_id: &str,
    session_id: &str,
) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "UPDATE sessions SET auth_time = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE id = ?1 AND user_id = ?2 \
           AND expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         RETURNING CAST(strftime('%s', auth_time) AS INTEGER)",
        (session_id, user_id),
        |row| row.get(0),
    )
    .optional()
}

/// At most `max` bytes, cut at a character boundary.
pub fn truncate(value: &str, max: usize) -> &str {
    if value.len() <= max {
        return value;
    }
    let end = (0..=max)
        .rev()
        .find(|&i| value.is_char_boundary(i))
        .unwrap_or(0);
    &value[..end]
}

/// A valid session for Account › Security › Devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
}

/// Valid sessions of the user, most recently active first.
pub fn list(conn: &Connection, user_id: &str) -> rusqlite::Result<Vec<Listed>> {
    conn.prepare(
        "SELECT id, user_agent, ip, created_at, last_seen_at FROM sessions \
         WHERE user_id = ?1 AND expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         ORDER BY last_seen_at DESC, rowid DESC",
    )?
    .query_map([user_id], |row| {
        Ok(Listed {
            id: row.get(0)?,
            user_agent: row.get(1)?,
            ip: row.get(2)?,
            created_at: row.get(3)?,
            last_seen_at: row.get(4)?,
        })
    })?
    .collect()
}

/// Whether the session still exists and has not expired – for changes that
/// follow a longer check in a transaction (change password, "Sign out all
/// others"): if the session ended in the meantime, the change does not apply.
pub fn is_valid(conn: &Connection, user_id: &str, session_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sessions WHERE id = ?1 AND user_id = ?2 \
                        AND expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        (session_id, user_id),
        |row| row.get(0),
    )
}

/// The sessions among `ids` that are still valid: not expired, their user
/// active (as [`authenticate`] checks).
pub fn valid_among(
    conn: &Connection,
    ids: &[String],
) -> rusqlite::Result<std::collections::HashSet<String>> {
    let ids = serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_owned());
    conn.prepare(
        "SELECT s.id FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.id IN (SELECT value FROM json_each(?1)) \
           AND s.expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
           AND u.status = 'active'",
    )?
    .query_map([ids], |row| row.get(0))?
    .collect()
}

/// Ends the user's sessions beyond `max`, least recently used first (so
/// expired ones not yet cleaned up go before all valid ones); `newest` always
/// stays. Returns the number deleted. No audit log entry: the sign-in that
/// triggers it is already logged.
pub fn trim(conn: &Connection, user_id: &str, newest: &str, max: u32) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM sessions WHERE user_id = ?1 AND id <> ?2 AND id NOT IN ( \
           SELECT id FROM sessions WHERE user_id = ?1 AND id <> ?2 \
           ORDER BY last_seen_at DESC, rowid DESC LIMIT ?3)",
        (user_id, newest, max.saturating_sub(1)),
    )
}

/// Deletes expired sessions; valid ones stay.
pub fn delete_expired(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM sessions WHERE expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        [],
    )
}

/// Ends a session of the user; `false` if it does not exist (any more).
pub fn delete(conn: &Connection, user_id: &str, session_id: &str) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "DELETE FROM sessions WHERE id = ?1 AND user_id = ?2",
        (session_id, user_id),
    )?;
    Ok(n > 0)
}

/// Ends all sessions of the user except `keep` (changing the password, "Sign
/// out all others"). Returns the number of still valid ones: expired rows not
/// yet cleaned up are deleted too but do not count as a signed-in device.
pub fn delete_for_user(
    conn: &Connection,
    user_id: &str,
    keep: Option<&str>,
) -> rusqlite::Result<usize> {
    let active = conn
        .prepare(
            "DELETE FROM sessions WHERE user_id = ?1 AND id IS NOT ?2 \
             RETURNING expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )?
        .query_map((user_id, keep), |row| row.get::<_, bool>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(active.into_iter().filter(|active| *active).count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::session::{self, Client},
        db::repo::users::tests::{add, conn},
    };

    fn token_hash(conn: &Connection, user: &str) -> ([u8; 32], String) {
        let started = session::start(conn, user, &Client::default(), true).unwrap();
        let hash: Vec<u8> = conn
            .query_row(
                "SELECT token_hash FROM sessions WHERE id = ?1",
                [&started.id],
                |r| r.get(0),
            )
            .unwrap();
        (hash.try_into().unwrap(), started.id)
    }

    fn set(conn: &Connection, sql: &str, id: &str) {
        conn.execute(sql, [id]).unwrap();
    }

    #[test]
    fn authenticates_active_sessions_only() {
        let conn = conn();
        let user = add(&conn, "max", "max@example.com", Role::Admin);
        let (hash, id) = token_hash(&conn, &user);

        let found = authenticate(&conn, &hash).unwrap().unwrap();
        assert_eq!(found.user.id, user);
        assert_eq!(found.user.session_id, id);
        assert_eq!(found.user.role, Role::Admin);
        assert!(found.persistent);
        assert!(!found.touched, "a fresh session needs no extension");
        assert!(authenticate(&conn, &[0; 32]).unwrap().is_none());

        set(
            &conn,
            "UPDATE users SET status = 'locked' WHERE id = ?1",
            &user,
        );
        assert!(authenticate(&conn, &hash).unwrap().is_none());
        set(
            &conn,
            "UPDATE users SET status = 'active' WHERE id = ?1",
            &user,
        );

        set(
            &conn,
            "UPDATE sessions SET expires_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1",
            &id,
        );
        assert!(authenticate(&conn, &hash).unwrap().is_none());
    }

    #[test]
    fn auth_time_in_unix_seconds_and_renewed_by_reauthentication() {
        let conn = conn();
        let user = add(&conn, "max", "max@example.com", Role::User);
        let other = add(&conn, "eva", "eva@example.com", Role::User);
        let (hash, id) = token_hash(&conn, &user);
        set(
            &conn,
            "UPDATE sessions SET auth_time = '2026-11-25T10:00:00.000Z' WHERE id = ?1",
            &id,
        );
        let found = authenticate(&conn, &hash).unwrap().unwrap();
        assert_eq!(found.user.auth_time, 1_795_600_800);

        assert_eq!(reauthenticate(&conn, &other, &id).unwrap(), None);
        let renewed = reauthenticate(&conn, &user, &id).unwrap().unwrap();
        assert!(renewed > 1_795_600_800, "{renewed}");
        let found = authenticate(&conn, &hash).unwrap().unwrap();
        assert_eq!(found.user.auth_time, renewed);
        assert_eq!(found.user.session_id, id, "same session");

        set(
            &conn,
            "UPDATE sessions SET expires_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1",
            &id,
        );
        assert_eq!(reauthenticate(&conn, &user, &id).unwrap(), None);
    }

    #[test]
    fn touch_extends_session_and_last_seen() {
        let conn = conn();
        let user = add(&conn, "max", "max@example.com", Role::User);
        let (hash, id) = token_hash(&conn, &user);
        set(
            &conn,
            "UPDATE sessions SET last_seen_at = '2000-01-01T00:00:00.000Z', \
                                 expires_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+1 hour') \
             WHERE id = ?1",
            &id,
        );

        assert!(authenticate(&conn, &hash).unwrap().unwrap().touched);
        let (days, seen): (f64, Option<String>) = conn
            .query_row(
                "SELECT julianday(s.expires_at) - julianday('now'), u.last_seen_at \
                 FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!((days - 30.0).abs() < 0.01, "{days}");
        assert!(seen.is_some());
        // Not again within a minute.
        assert!(!authenticate(&conn, &hash).unwrap().unwrap().touched);
    }

    #[test]
    fn failed_touch_keeps_request_authenticated() {
        let conn = conn();
        let user = add(&conn, "max", "max@example.com", Role::User);
        let (hash, id) = token_hash(&conn, &user);
        set(
            &conn,
            "UPDATE sessions SET last_seen_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1",
            &id,
        );
        conn.execute_batch(
            "CREATE TRIGGER no_touch BEFORE UPDATE ON users \
             BEGIN SELECT raise(ABORT, 'blocked'); END;",
        )
        .unwrap();

        let found = authenticate(&conn, &hash).unwrap().unwrap();
        assert!(!found.touched);
        // The session stayed unchanged too (one transaction).
        let seen: String = conn
            .query_row(
                "SELECT last_seen_at FROM sessions WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(seen, "2000-01-01T00:00:00.000Z");
    }

    #[test]
    fn deletes_single_and_all_but_one() {
        let conn = conn();
        let max = add(&conn, "max", "max@example.com", Role::User);
        let eva = add(&conn, "eva", "eva@example.com", Role::User);
        let (_, a) = token_hash(&conn, &max);
        let (_, b) = token_hash(&conn, &max);
        token_hash(&conn, &max);
        let (_, other) = token_hash(&conn, &eva);

        assert!(!delete(&conn, &eva, &a).unwrap(), "foreign session");
        assert!(delete(&conn, &max, &a).unwrap());
        assert!(!delete(&conn, &max, &a).unwrap());
        assert_eq!(delete_for_user(&conn, &max, Some(&b)).unwrap(), 1);
        // Expired but not yet cleaned up: deleted, not counted.
        let (_, expired) = token_hash(&conn, &max);
        set(
            &conn,
            "UPDATE sessions SET expires_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1",
            &expired,
        );
        assert_eq!(delete_for_user(&conn, &max, None).unwrap(), 1);
        let left: Vec<String> = conn
            .prepare("SELECT id FROM sessions ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(left, [other]);
    }

    #[test]
    fn user_agent_is_truncated() {
        let conn = conn();
        let user = add(&conn, "max", "max@example.com", Role::User);
        let long = format!("Mozilla/5.0 {}", "é".repeat(600));
        let client = Client {
            user_agent: Some(&long),
            ..Client::default()
        };
        let started = session::start(&conn, &user, &client, true).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT user_agent FROM sessions WHERE id = ?1",
                [&started.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(stored.len() <= MAX_USER_AGENT && stored.len() >= MAX_USER_AGENT - 1);
        assert!(long.starts_with(&stored));
        assert_eq!(truncate("short", 10), "short");
    }

    #[test]
    fn lists_valid_sessions_and_cleans_up_expired() {
        let conn = conn();
        let max = add(&conn, "max", "max@example.com", Role::User);
        let eva = add(&conn, "eva", "eva@example.com", Role::User);
        let client = Client {
            user_agent: Some("Firefox"),
            ip: Some("198.51.100.56".parse().unwrap()),
            lang: None,
        };
        let a = session::start(&conn, &max, &client, false).unwrap().id;
        let (_, b) = token_hash(&conn, &max);
        let (_, expired) = token_hash(&conn, &max);
        let (_, other) = token_hash(&conn, &eva);
        set(
            &conn,
            "UPDATE sessions SET expires_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1",
            &expired,
        );
        set(
            &conn,
            "UPDATE sessions SET last_seen_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1",
            &a,
        );

        let listed = list(&conn, &max).unwrap();
        let ids: Vec<&str> = listed.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, [b.as_str(), a.as_str()], "most recently active first");
        assert_eq!(listed[1].ip.as_deref(), Some("198.51.100.56"));
        assert_eq!(listed[1].user_agent.as_deref(), Some("Firefox"));
        assert!(is_valid(&conn, &max, &a).unwrap());
        assert!(!is_valid(&conn, &eva, &a).unwrap(), "foreign session");
        assert!(!is_valid(&conn, &max, &expired).unwrap());

        assert_eq!(delete_expired(&conn).unwrap(), 1);
        assert_eq!(delete_expired(&conn).unwrap(), 0);
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 3);
        assert!(is_valid(&conn, &eva, &other).unwrap());
    }

    #[test]
    fn new_session_ends_the_least_recently_used_beyond_the_limit() {
        let conn = conn();
        let max = add(&conn, "max", "max@example.com", Role::User);
        let kai = add(&conn, "kai", "kai@example.com", Role::User);
        let other = session::start(&conn, &kai, &Client::default(), true)
            .unwrap()
            .id;
        let ids: Vec<String> = (0..MAX_PER_USER)
            .map(|_| {
                session::start(&conn, &max, &Client::default(), true)
                    .unwrap()
                    .id
            })
            .collect();
        // The second was inactive the longest, the first was active just now.
        set(
            &conn,
            "UPDATE sessions SET last_seen_at = '2026-01-01T00:00:00.000Z' WHERE id = ?1",
            &ids[1],
        );
        set(
            &conn,
            "UPDATE sessions SET last_seen_at = '2099-01-01T00:00:00.000Z' WHERE id = ?1",
            &ids[0],
        );
        let ids_of = |user: &str| -> Vec<String> {
            list(&conn, user)
                .unwrap()
                .into_iter()
                .map(|s| s.id)
                .collect()
        };
        assert_eq!(ids_of(&max).len(), 20);

        let newest = session::start(&conn, &max, &Client::default(), true)
            .unwrap()
            .id;
        let left = ids_of(&max);
        assert_eq!(left.len(), 20);
        assert!(!left.contains(&ids[1]), "the least recently used one ends");
        assert!(left.contains(&ids[0]) && left.contains(&newest));
        // Other users stay untouched.
        assert_eq!(ids_of(&kai), [other]);
    }

    #[test]
    fn deleting_user_removes_sessions() {
        let conn = conn();
        let user = add(&conn, "max", "max@example.com", Role::User);
        token_hash(&conn, &user);
        conn.execute("DELETE FROM users WHERE id = ?1", [&user])
            .unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}
