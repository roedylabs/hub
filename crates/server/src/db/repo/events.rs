//! Tables `events` and `event_delivery`: the events for the apps
//! (docs/app-contract.md › Events) as an outbox, and how far their delivery
//! got.
//!
//! **Writing:** the functions that change users, sessions and access
//! (`users`, `sessions`, `app_access`, `settings::save`) append their events
//! here in the same transaction as the change ([`db::atomic`]), so that an
//! event exists exactly when its change does. User and session events go to
//! every paired app (contract § 10.2: the app may hold data from earlier
//! access), `access.changed` only to its app; pending apps get none. Each
//! app has its own `seq`, counted in `event_delivery.last_seq` so that it
//! never repeats after old events are deleted.
//!
//! **Reading:** the delivery job (`events`) takes the next event after
//! `delivered_seq` ([`next`]) and records the outcome ([`delivered`],
//! [`failed`]); `GET /internal/v1/events` reads with [`page`]; the
//! retention deletes with [`delete_older_than`]. Removing an app deletes its
//! events and their state (`ON DELETE CASCADE`).

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension as _};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    db::{self, repo::users},
    settings,
};

/// Event types (contract § 10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    UserUpdated,
    UserLocked,
    UserUnlocked,
    UserDeleted,
    SessionRevoked,
    AccessChanged,
}

impl Kind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserUpdated => "user.updated",
            Self::UserLocked => "user.locked",
            Self::UserUnlocked => "user.unlocked",
            Self::UserDeleted => "user.deleted",
            Self::SessionRevoked => "session.revoked",
            Self::AccessChanged => "access.changed",
        }
    }
}

/// Why a session ended (`reason` of `session.revoked`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Signed out, also by a new sign-in that replaced the browser's session.
    Logout,
    /// Ended from Account › Security, by "Sign out all others" or because
    /// the user has too many sessions.
    Revoked,
    /// Expired without activity.
    Expired,
    /// The password was changed or reset.
    PasswordChanged,
    /// The user was locked.
    Locked,
}

impl Reason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Logout => "logout",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::PasswordChanged => "password_changed",
            Self::Locked => "locked",
        }
    }
}

/// An error outside SQLite (no random source, unreadable settings) as a
/// `rusqlite::Error`, so that the changing functions keep their signature.
fn foreign(error: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(error.into())
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// Appends an event to the app's stream; returns its `seq`.
pub fn append(conn: &Connection, app_id: &str, kind: Kind, data: &Value) -> rusqlite::Result<u64> {
    db::atomic(conn, |conn| {
        let seq: i64 = conn.query_row(
            "INSERT INTO event_delivery (app_id, last_seq) VALUES (?1, 1) \
             ON CONFLICT (app_id) DO UPDATE SET last_seq = last_seq + 1 \
             RETURNING last_seq",
            [app_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO events (id, app_id, seq, type, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                db::new_id().map_err(foreign)?,
                app_id,
                seq,
                kind.as_str(),
                data.to_string(),
            ),
        )?;
        Ok(to_u64(seq))
    })
}

/// IDs of the paired apps, which get events.
pub fn paired_apps(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    conn.prepare("SELECT id FROM apps WHERE state = 'paired' ORDER BY id")?
        .query_map([], |row| row.get(0))?
        .collect()
}

/// Appends the event to every paired app; returns how many got it.
pub fn to_paired_apps(conn: &Connection, kind: Kind, data: &Value) -> rusqlite::Result<usize> {
    db::atomic(conn, |conn| {
        let apps = paired_apps(conn)?;
        for app in &apps {
            append(conn, app, kind, data)?;
        }
        Ok(apps.len())
    })
}

pub fn user_locked(conn: &Connection, sub: &str) -> rusqlite::Result<usize> {
    to_paired_apps(conn, Kind::UserLocked, &json!({ "sub": sub }))
}

pub fn user_unlocked(conn: &Connection, sub: &str) -> rusqlite::Result<usize> {
    to_paired_apps(conn, Kind::UserUnlocked, &json!({ "sub": sub }))
}

pub fn user_deleted(conn: &Connection, sub: &str) -> rusqlite::Result<usize> {
    to_paired_apps(conn, Kind::UserDeleted, &json!({ "sub": sub }))
}

pub fn session_revoked(
    conn: &Connection,
    sub: &str,
    sid: &str,
    reason: Reason,
) -> rusqlite::Result<usize> {
    to_paired_apps(
        conn,
        Kind::SessionRevoked,
        &json!({ "sub": sub, "sid": sid, "reason": reason.as_str() }),
    )
}

/// `access.changed` for one app: with `app_role` the user has access with
/// that role, without none.
pub fn access_changed(
    conn: &Connection,
    app_id: &str,
    sub: &str,
    app_role: Option<&str>,
) -> rusqlite::Result<u64> {
    append(
        conn,
        app_id,
        Kind::AccessChanged,
        &json!({ "sub": sub, "access": app_role.is_some(), "app_role": app_role }),
    )
}

/// What `user.updated` carries: the user's current values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserData {
    pub sub: String,
    pub username: String,
    pub email: String,
    pub name: String,
    pub role: String,
    /// `en` or `de`, as in the identity.
    pub locale: String,
}

/// The user's values as an app sees them; `None` for an unknown user.
pub fn user_data(conn: &Connection, id: &str) -> rusqlite::Result<Option<UserData>> {
    let Some(found) = users::credentials_by_id(conn, id)? else {
        return Ok(None);
    };
    let locale = settings::lang_of(conn, id).map_err(foreign)?;
    let user = found.user;
    Ok(Some(UserData {
        sub: user.id,
        username: user.username,
        email: user.email,
        name: user.display_name,
        role: user.role.as_str().to_owned(),
        locale: locale.code().to_owned(),
    }))
}

/// `user.updated` to every paired app if what an app sees of the user
/// differs from `before` (taken with [`user_data`] before the change);
/// `true` if so.
pub fn user_updated_since(conn: &Connection, before: Option<&UserData>) -> rusqlite::Result<bool> {
    let Some(before) = before else {
        return Ok(false);
    };
    let Some(after) = user_data(conn, &before.sub)? else {
        return Ok(false);
    };
    if &after == before {
        return Ok(false);
    }
    let data = serde_json::to_value(&after).map_err(|error| foreign(error.into()))?;
    to_paired_apps(conn, Kind::UserUpdated, &data)?;
    Ok(true)
}

/// Runs `change` and reports it with `user.updated` if it changed what the
/// apps see of user `id`. One savepoint for both.
pub fn tracking_user<T, E: From<rusqlite::Error>>(
    conn: &Connection,
    id: &str,
    change: impl FnOnce(&Connection) -> Result<T, E>,
) -> Result<T, E> {
    db::atomic(conn, |conn| {
        let before = user_data(conn, id)?;
        let result = change(conn)?;
        user_updated_since(conn, before.as_ref())?;
        Ok(result)
    })
}

/// A stored event.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    /// The event ID, `jti`.
    pub id: String,
    pub seq: u64,
    pub kind: String,
    pub data: Value,
}

fn stored(row: &rusqlite::Row<'_>) -> rusqlite::Result<Stored> {
    let data: String = row.get(3)?;
    Ok(Stored {
        id: row.get(0)?,
        seq: to_u64(row.get(1)?),
        kind: row.get(2)?,
        data: serde_json::from_str(&data).unwrap_or(Value::Null),
    })
}

const STORED: &str = "SELECT id, seq, type, data FROM events";

/// What the delivery of an app does next ([`next`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Next {
    /// The app is gone or no longer paired: delivery ends.
    Gone,
    /// Nothing to deliver.
    Idle,
    /// The hub gave up after failures and waits until the app is back
    /// ([`resume`]).
    Suspended,
    /// The next attempt is due after this time.
    Wait(Duration),
    /// Deliver this event now; `attempts` failed before.
    Deliver {
        slug: String,
        internal_url: String,
        event: Stored,
        attempts: u32,
    },
}

/// The next step of the delivery to app `app_id`.
pub fn next(conn: &Connection, app_id: &str) -> rusqlite::Result<Next> {
    let state = conn
        .query_row(
            "SELECT a.slug, a.internal_url, a.state = 'paired', \
                    coalesce(d.delivered_seq, 0), coalesce(d.attempts, 0), \
                    d.retry_at IS NULL, \
                    CAST(max(0, (julianday(d.retry_at) - julianday('now')) * 86400000) \
                         AS INTEGER) \
             FROM apps a LEFT JOIN event_delivery d ON d.app_id = a.id WHERE a.id = ?1",
            [app_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .optional()?;
    let Some((slug, internal_url, paired, delivered, attempts, no_retry, wait_ms)) = state else {
        return Ok(Next::Gone);
    };
    if !paired {
        return Ok(Next::Gone);
    }
    let event = conn
        .query_row(
            &format!("{STORED} WHERE app_id = ?1 AND seq > ?2 ORDER BY seq LIMIT 1"),
            (app_id, delivered),
            stored,
        )
        .optional()?;
    let Some(event) = event else {
        return Ok(Next::Idle);
    };
    if attempts > 0 && no_retry {
        return Ok(Next::Suspended);
    }
    if let Some(ms) = wait_ms.filter(|ms| *ms > 0) {
        return Ok(Next::Wait(Duration::from_millis(to_u64(ms))));
    }
    Ok(Next::Deliver {
        slug,
        internal_url,
        event,
        attempts: u32::try_from(attempts).unwrap_or(u32::MAX),
    })
}

/// The app acknowledged event `seq`: the next one follows at once.
pub fn delivered(conn: &Connection, app_id: &str, seq: u64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE event_delivery SET delivered_seq = max(delivered_seq, min(?2, last_seq)), \
                attempts = 0, retry_at = NULL, failing_since = NULL, last_error = NULL \
         WHERE app_id = ?1",
        (app_id, to_i64(seq)),
    )?;
    Ok(())
}

/// SQLite date modifier for `duration` from now, e.g. `+10.000 seconds`.
fn modifier(sign: char, duration: Duration) -> String {
    format!("{sign}{:.3} seconds", duration.as_secs_f64())
}

/// Outcome of a failed attempt ([`failed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Failure {
    /// Failed attempts in this series, this one included.
    pub attempts: u32,
    /// The hub gave up: the next attempt would fall after `give_up` from
    /// the first failure.
    pub suspended: bool,
}

/// Records a failed attempt at event `seq`: the next one after
/// `delay(attempts)`, unless that falls more than `give_up` after the first
/// failure of the series – then delivery waits for [`resume`]. Nothing
/// happens if the event was acknowledged meanwhile (catch-up); then `None`.
pub fn failed(
    conn: &Connection,
    app_id: &str,
    seq: u64,
    error: &str,
    delay: impl FnOnce(u32) -> Duration,
    give_up: Duration,
) -> rusqlite::Result<Option<Failure>> {
    db::atomic(conn, |conn| {
        let attempts: Option<i64> = conn
            .query_row(
                "UPDATE event_delivery SET attempts = attempts + 1, \
                        failing_since = coalesce(failing_since, \
                                                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
                        last_error = ?3 \
                 WHERE app_id = ?1 AND delivered_seq < ?2 RETURNING attempts",
                (app_id, to_i64(seq), error),
                |row| row.get(0),
            )
            .optional()?;
        let Some(attempts) = attempts else {
            return Ok(None);
        };
        let attempts = u32::try_from(attempts).unwrap_or(u32::MAX);
        let suspended: bool = conn.query_row(
            "UPDATE event_delivery SET retry_at = \
                CASE WHEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?2) \
                          > strftime('%Y-%m-%dT%H:%M:%fZ', failing_since, ?3) \
                     THEN NULL \
                     ELSE strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?2) END \
             WHERE app_id = ?1 RETURNING retry_at IS NULL",
            (
                app_id,
                modifier('+', delay(attempts)),
                modifier('+', give_up),
            ),
            |row| row.get(0),
        )?;
        Ok(Some(Failure {
            attempts,
            suspended,
        }))
    })
}

/// Lets delivery try again at once: after it gave up (`only_suspended`), or
/// also during a backoff (the app showed it is back). `true` if anything
/// changed.
pub fn resume(conn: &Connection, app_id: &str, only_suspended: bool) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE event_delivery SET attempts = 0, retry_at = NULL, failing_since = NULL \
         WHERE app_id = ?1 AND attempts > 0 AND (?2 = 0 OR retry_at IS NULL)",
        (app_id, only_suspended),
    )?;
    Ok(n > 0)
}

/// The app processed everything up to `seq` (it reads the events after it):
/// delivery continues after it, at once. `true` if that moved it on.
pub fn acknowledge(conn: &Connection, app_id: &str, seq: u64) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE event_delivery SET delivered_seq = min(?2, last_seq), attempts = 0, \
                retry_at = NULL, failing_since = NULL \
         WHERE app_id = ?1 AND delivered_seq < min(?2, last_seq)",
        (app_id, to_i64(seq)),
    )?;
    Ok(n > 0)
}

/// Delivery state of an app, for tests and the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub last_seq: u64,
    pub delivered_seq: u64,
    pub attempts: u32,
    pub retry_at: Option<String>,
    pub failing_since: Option<String>,
    pub last_error: Option<String>,
}

const STATE: &str = "SELECT last_seq, delivered_seq, attempts, retry_at, failing_since, \
                            last_error, app_id \
                     FROM event_delivery";

fn delivery_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<State> {
    Ok(State {
        last_seq: to_u64(row.get(0)?),
        delivered_seq: to_u64(row.get(1)?),
        attempts: u32::try_from(row.get::<_, i64>(2)?).unwrap_or(u32::MAX),
        retry_at: row.get(3)?,
        failing_since: row.get(4)?,
        last_error: row.get(5)?,
    })
}

pub fn state(conn: &Connection, app_id: &str) -> rusqlite::Result<Option<State>> {
    conn.query_row(
        &format!("{STATE} WHERE app_id = ?1"),
        [app_id],
        delivery_state,
    )
    .optional()
}

/// The delivery state of every app with events, by app ID.
pub fn states(conn: &Connection) -> rusqlite::Result<std::collections::HashMap<String, State>> {
    conn.prepare(STATE)?
        .query_map([], |row| Ok((row.get(6)?, delivery_state(row)?)))?
        .collect()
}

/// One page of `GET /internal/v1/events` ([`page`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    /// Events after the requested `seq` were deleted already; `resume_after
    /// + 1` is the oldest one kept.
    Expired { resume_after: u64 },
    /// The events after the requested `seq`, oldest first.
    Events { events: Vec<Stored>, more: bool },
}

/// At most `limit` events of the app after `after` (contract § 10.3).
pub fn page(conn: &Connection, app_id: &str, after: u64, limit: usize) -> rusqlite::Result<Page> {
    let (last, oldest): (i64, Option<i64>) = conn.query_row(
        "SELECT coalesce((SELECT last_seq FROM event_delivery WHERE app_id = ?1), 0), \
                (SELECT min(seq) FROM events WHERE app_id = ?1)",
        [app_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let (last, oldest_kept) = (to_u64(last), oldest.map_or(to_u64(last) + 1, to_u64));
    if after < last && after + 1 < oldest_kept {
        return Ok(Page::Expired {
            resume_after: oldest_kept - 1,
        });
    }
    let mut events: Vec<Stored> = conn
        .prepare(&format!(
            "{STORED} WHERE app_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3"
        ))?
        .query_map(
            (
                app_id,
                to_i64(after),
                i64::try_from(limit).unwrap_or(i64::MAX) + 1,
            ),
            stored,
        )?
        .collect::<rusqlite::Result<_>>()?;
    let more = events.len() > limit;
    events.truncate(limit);
    Ok(Page::Events { events, more })
}

/// Deletes events older than `keep`, delivered or not; returns how many.
/// Per app only the events before its oldest one to keep: if the clock went
/// back, a later `seq` can carry an older `created_at`, and deleting it
/// alone would leave a gap that [`page`] cannot see. So what remains is
/// always every event from some `seq` on. The boundary is looked up once
/// per app, and the deletion goes by `(app_id, seq)`, so a run stays linear
/// in the number of events.
pub fn delete_older_than(conn: &Connection, keep: Duration) -> rusqlite::Result<usize> {
    db::atomic(conn, |conn| {
        let cutoff: String = conn.query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)",
            [modifier('-', keep)],
            |row| row.get(0),
        )?;
        // Apps with something to delete, and the first `seq` each keeps
        // (none: everything goes).
        let bounds: Vec<(String, Option<i64>)> = conn
            .prepare(
                "SELECT old.app_id, \
                        (SELECT min(young.seq) FROM events young \
                         WHERE young.app_id = old.app_id AND young.created_at >= ?1) \
                 FROM (SELECT DISTINCT app_id FROM events WHERE created_at < ?1) old",
            )?
            .query_map([&cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut deleted = 0;
        for (app_id, keep_from) in bounds {
            deleted += conn.execute(
                "DELETE FROM events WHERE app_id = ?1 AND seq < ?2",
                (app_id, keep_from.unwrap_or(i64::MAX)),
            )?;
        }
        Ok(deleted)
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::repo::{
        apps::{self, tests::manifest},
        users::tests::conn,
    };

    /// A paired app; returns its ID.
    pub(crate) fn paired(conn: &Connection, slug: &str, hash: u8) -> String {
        let id = apps::insert(conn, slug, "http://app.test").unwrap();
        apps::pair(conn, &id, &manifest(), &[hash; 32]).unwrap();
        id
    }

    /// The app's events as `(seq, type, data)`.
    pub(crate) fn events_of(conn: &Connection, app_id: &str) -> Vec<(u64, String, Value)> {
        match page(conn, app_id, 0, 1000).unwrap() {
            Page::Events { events, .. } => events
                .into_iter()
                .map(|e| (e.seq, e.kind, e.data))
                .collect(),
            Page::Expired { .. } => panic!("expired"),
        }
    }

    #[test]
    fn sequences_per_app_and_never_repeated() {
        let conn = conn();
        let mail = paired(&conn, "mail", 1);
        let notes = paired(&conn, "notes", 2);
        let pending = apps::insert(&conn, "later", "http://later.test").unwrap();
        assert_eq!(user_locked(&conn, "u1").unwrap(), 2, "paired apps only");
        assert_eq!(access_changed(&conn, &mail, "u1", Some("user")).unwrap(), 2);
        assert_eq!(
            session_revoked(&conn, "u1", "s1", Reason::Logout).unwrap(),
            2
        );
        assert_eq!(
            events_of(&conn, &mail),
            [
                (1, "user.locked".into(), json!({ "sub": "u1" })),
                (
                    2,
                    "access.changed".into(),
                    json!({ "sub": "u1", "access": true, "app_role": "user" })
                ),
                (
                    3,
                    "session.revoked".into(),
                    json!({ "sub": "u1", "sid": "s1", "reason": "logout" })
                ),
            ]
        );
        assert_eq!(
            events_of(&conn, &notes)
                .iter()
                .map(|e| e.0)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert!(events_of(&conn, &pending).is_empty());

        // Old events go, the counter stays.
        conn.execute(
            "UPDATE events SET created_at = '2000-01-01T00:00:00.000Z' WHERE app_id = ?1",
            [&mail],
        )
        .unwrap();
        assert_eq!(
            delete_older_than(&conn, Duration::from_secs(30 * 86_400)).unwrap(),
            3
        );
        assert_eq!(user_unlocked(&conn, "u1").unwrap(), 2);
        assert_eq!(events_of_after(&conn, &mail, 3), [4]);
        assert_eq!(
            page(&conn, &mail, 0, 10).unwrap(),
            Page::Expired { resume_after: 3 }
        );
        assert_eq!(
            page(&conn, &mail, 2, 10).unwrap(),
            Page::Expired { resume_after: 3 }
        );

        // Removing the app removes its events and state.
        apps::delete(&conn, &mail).unwrap();
        assert_eq!(state(&conn, &mail).unwrap(), None);
        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE app_id = ?1",
                [&mail],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn the_retention_never_leaves_a_gap() {
        let conn = conn();
        let mail = paired(&conn, "mail", 1);
        let notes = paired(&conn, "notes", 2);
        for n in 1..=5 {
            user_locked(&conn, &format!("u{n}")).unwrap();
        }
        // The clock went back: seq 2 and 4 look older than seq 1 and 3.
        conn.execute(
            "UPDATE events SET created_at = CASE \
               WHEN seq IN (2, 4) THEN '2000-01-01T00:00:00.000Z' \
               WHEN seq = 1 THEN '2000-01-02T00:00:00.000Z' \
               ELSE created_at END WHERE app_id = ?1",
            [&mail],
        )
        .unwrap();
        let keep = Duration::from_secs(30 * 86_400);
        // Only 1 and 2 go: 4 stays with 3, the oldest event to keep.
        assert_eq!(delete_older_than(&conn, keep).unwrap(), 2);
        assert_eq!(events_of_after(&conn, &mail, 2), [3, 4, 5]);
        assert_eq!(
            page(&conn, &mail, 0, 10).unwrap(),
            Page::Expired { resume_after: 2 }
        );
        // Another app's events are untouched; all old: all go.
        assert_eq!(events_of_after(&conn, &notes, 0), [1, 2, 3, 4, 5]);
        conn.execute(
            "UPDATE events SET created_at = '2000-01-01T00:00:00.000Z' WHERE app_id = ?1",
            [&notes],
        )
        .unwrap();
        assert_eq!(delete_older_than(&conn, keep).unwrap(), 5);
        assert_eq!(
            page(&conn, &notes, 0, 10).unwrap(),
            Page::Expired { resume_after: 5 }
        );
    }

    fn events_of_after(conn: &Connection, app_id: &str, after: u64) -> Vec<u64> {
        match page(conn, app_id, after, 10).unwrap() {
            Page::Events { events, .. } => events.iter().map(|e| e.seq).collect(),
            Page::Expired { .. } => panic!("expired"),
        }
    }

    #[test]
    fn changes_report_themselves_in_their_transaction() {
        use crate::{
            auth::session::{self, Client},
            db::repo::{
                sessions,
                users::{self, Role, tests::add},
            },
            settings::Language,
        };

        let mut conn = conn();
        let mail = paired(&conn, "mail", 1);
        let max = add(&conn, "max", "max@example.com", Role::User);
        // Straight from the table: the test deletes events in between.
        let kinds = |conn: &Connection| -> Vec<String> {
            conn.prepare(&format!("{STORED} WHERE app_id = ?1 ORDER BY seq"))
                .unwrap()
                .query_map([&mail], stored)
                .unwrap()
                .map(|event| {
                    let event = event.unwrap();
                    (event.kind, event.data)
                })
                .map(|(kind, data)| match data["reason"].as_str() {
                    Some(reason) => format!("{kind} {reason}"),
                    None => kind,
                })
                .collect()
        };
        assert_eq!(kinds(&conn), ["access.changed"]);

        // The email address; the sign-in language once it decides the
        // language (`system`), and only when it changes.
        users::set_email(&conn, &max, "max@example.org").unwrap();
        let system = crate::settings::Settings {
            lang: Language::System,
            ..crate::settings::Settings::default()
        };
        crate::settings::save(&conn, &max, 0, &system).unwrap();
        let german = Client {
            lang: Some(Language::German),
            ..Client::default()
        };
        session::start(&conn, &max, &german, true).unwrap();
        session::start(&conn, &max, &german, true).unwrap();
        let updates: Vec<Value> = events_of(&conn, &mail)
            .into_iter()
            .filter(|e| e.1 == "user.updated")
            .map(|e| e.2)
            .collect();
        assert_eq!(updates.len(), 2);
        assert_eq!(
            (&updates[0]["email"], &updates[0]["locale"]),
            (&json!("max@example.org"), &json!("en"))
        );
        assert_eq!(updates[1]["locale"], "de");

        // Too many sessions: the oldest ends as revoked; an expired one
        // that ends with the others counts as expired.
        conn.execute("DELETE FROM events", []).unwrap();
        for _ in 0..sessions::MAX_PER_USER - 1 {
            session::start(&conn, &max, &Client::default(), true).unwrap();
        }
        assert_eq!(kinds(&conn), ["session.revoked revoked"]);
        conn.execute("DELETE FROM events", []).unwrap();
        conn.execute(
            "UPDATE sessions SET expires_at = '2000-01-01T00:00:00.000Z' \
             WHERE id = (SELECT min(id) FROM sessions)",
            [],
        )
        .unwrap();
        sessions::delete_for_user(&conn, &max, None, Reason::PasswordChanged).unwrap();
        let mut ended = kinds(&conn);
        ended.sort();
        ended.dedup();
        assert_eq!(
            ended,
            [
                "session.revoked expired",
                "session.revoked password_changed"
            ]
        );

        // A rolled-back change leaves no event.
        conn.execute("DELETE FROM events", []).unwrap();
        let tx = conn.transaction().unwrap();
        assert!(users::lock(&tx, &max).unwrap());
        assert_eq!(kinds(&tx), ["user.locked"]);
        drop(tx);
        assert!(kinds(&conn).is_empty());
        // The counter went back with it: 4 events, then 20 ended sessions.
        assert_eq!(state(&conn, &mail).unwrap().unwrap().last_seq, 4 + 20);
    }

    #[test]
    fn pages_through_the_events() {
        let conn = conn();
        let mail = paired(&conn, "mail", 1);
        assert_eq!(
            page(&conn, &mail, 0, 10).unwrap(),
            Page::Events {
                events: vec![],
                more: false
            }
        );
        for n in 0..5 {
            user_locked(&conn, &format!("u{n}")).unwrap();
        }
        let Page::Events { events, more } = page(&conn, &mail, 1, 2).unwrap() else {
            panic!("expired");
        };
        assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), [2, 3]);
        assert!(more);
        let Page::Events { events, more } = page(&conn, &mail, 3, 2).unwrap() else {
            panic!("expired");
        };
        assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), [4, 5]);
        assert!(!more);
        // At or after the end: nothing, never 410.
        assert_eq!(events_of_after(&conn, &mail, 5), Vec::<u64>::new());
        assert_eq!(events_of_after(&conn, &mail, 99), Vec::<u64>::new());
        // Everything deleted: before the end is expired.
        conn.execute("DELETE FROM events", []).unwrap();
        assert_eq!(
            page(&conn, &mail, 4, 10).unwrap(),
            Page::Expired { resume_after: 5 }
        );
        assert_eq!(events_of_after(&conn, &mail, 5), Vec::<u64>::new());
    }

    #[test]
    fn delivery_goes_in_order_with_backoff() {
        let conn = conn();
        let mail = paired(&conn, "mail", 1);
        assert_eq!(next(&conn, &mail).unwrap(), Next::Idle);
        assert_eq!(next(&conn, "unknown").unwrap(), Next::Gone);
        user_locked(&conn, "u1").unwrap();
        user_unlocked(&conn, "u1").unwrap();
        let Next::Deliver {
            slug,
            internal_url,
            event,
            attempts,
        } = next(&conn, &mail).unwrap()
        else {
            panic!("nothing to deliver");
        };
        assert_eq!(
            (slug.as_str(), internal_url.as_str(), event.seq, attempts),
            ("mail", "http://app.test", 1, 0)
        );

        // A failure waits for the backoff, still with the same event.
        let delay = |attempts| Duration::from_secs(u64::from(attempts) * 60);
        let failure = failed(
            &conn,
            &mail,
            1,
            "status 503",
            delay,
            Duration::from_secs(3600),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            failure,
            Failure {
                attempts: 1,
                suspended: false
            }
        );
        let Next::Wait(wait) = next(&conn, &mail).unwrap() else {
            panic!("no backoff");
        };
        assert!(
            wait > Duration::from_secs(55) && wait <= Duration::from_secs(60),
            "{wait:?}"
        );
        let state1 = state(&conn, &mail).unwrap().unwrap();
        assert_eq!(
            (state1.attempts, state1.last_error.as_deref()),
            (1, Some("status 503"))
        );
        // Due again (as if the time had passed).
        conn.execute(
            "UPDATE event_delivery SET retry_at = '2000-01-01T00:00:00.000Z'",
            [],
        )
        .unwrap();
        assert!(matches!(
            next(&conn, &mail).unwrap(),
            Next::Deliver { attempts: 1, ref event, .. } if event.seq == 1
        ));

        // Past the give-up time it waits for the app.
        conn.execute(
            "UPDATE event_delivery SET failing_since = '2000-01-01T00:00:00.000Z'",
            [],
        )
        .unwrap();
        let failure = failed(&conn, &mail, 1, "timeout", delay, Duration::from_secs(3600))
            .unwrap()
            .unwrap();
        assert_eq!(
            failure,
            Failure {
                attempts: 2,
                suspended: true
            }
        );
        assert_eq!(next(&conn, &mail).unwrap(), Next::Suspended);
        assert!(!resume(&conn, "unknown", true).unwrap());
        assert!(resume(&conn, &mail, true).unwrap());
        assert!(matches!(
            next(&conn, &mail).unwrap(),
            Next::Deliver { attempts: 0, .. }
        ));

        // Success moves on; a late failure of an acknowledged event is ignored.
        delivered(&conn, &mail, 1).unwrap();
        assert!(matches!(
            next(&conn, &mail).unwrap(),
            Next::Deliver { ref event, .. } if event.seq == 2
        ));
        assert_eq!(
            failed(&conn, &mail, 1, "late", delay, Duration::from_secs(3600)).unwrap(),
            None
        );
        // A catch-up acknowledges up to its `after`, never beyond the last.
        assert!(acknowledge(&conn, &mail, 99).unwrap());
        assert!(!acknowledge(&conn, &mail, 1).unwrap());
        assert_eq!(state(&conn, &mail).unwrap().unwrap().delivered_seq, 2);
        assert_eq!(next(&conn, &mail).unwrap(), Next::Idle);

        // A pending app gets no delivery.
        apps::delete(&conn, &mail).unwrap();
        assert_eq!(next(&conn, &mail).unwrap(), Next::Gone);
    }
}
