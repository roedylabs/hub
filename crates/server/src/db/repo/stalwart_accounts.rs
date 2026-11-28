//! Tables `stalwart_accounts` (users' Stalwart accounts) and
//! `stalwart_sign_ins` (sign-ins at a server under way). `api::stalwart`
//! checks the input; this module only reads and writes. Tokens appear here
//! only encrypted and only when writing.
//!
//! Taken over from Roedy Mail (`crates/server/src/db/repo/mail_accounts.rs`)
//! with the default account in the table (Mail kept it in the settings) and
//! without the storage alert (notifications come later).

use std::{collections::HashMap, time::Duration};

use anyhow::Context as _;
use rusqlite::{Connection, OptionalExtension as _, Row};

/// Number of colours an account can have.
pub const COLORS: i64 = 6;

/// Purposes for the AAD of the encrypted values (`crypto`): `secret_enc`
/// and `access_token_enc` with the account's ID, the PKCE verifier of a
/// sign-in with its state hash.
pub const SECRET_PURPOSE: &str = "stalwart_accounts.secret";
pub const ACCESS_PURPOSE: &str = "stalwart_accounts.access_token";
pub const VERIFIER_PURPOSE: &str = "stalwart_sign_ins.verifier";

/// Sign-in kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    /// Sign-in at the server, code exchanged by the hub (refresh token).
    Oauth,
    /// The user's access token.
    Token,
}

impl AuthKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Oauth => "oauth",
            Self::Token => "token",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "oauth" => Some(Self::Oauth),
            "token" => Some(Self::Token),
            _ => None,
        }
    }
}

/// State of an account's sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ok,
    /// The server rejected the refresh token: "Sign in again".
    NeedsSignIn,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::NeedsSignIn => "needs_sign_in",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "ok" => Some(Self::Ok),
            "needs_sign_in" => Some(Self::NeedsSignIn),
            _ => None,
        }
    }
}

/// A row from `stalwart_accounts` without tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub user_id: String,
    /// Origin of the server.
    pub base_url: String,
    pub email: String,
    pub account_id: String,
    pub username: String,
    pub auth_kind: AuthKind,
    pub state: State,
    pub color: i64,
    pub is_default: bool,
    pub session_url: String,
    /// The JMAP session at the last sign-in.
    pub session_json: String,
    pub last_check_at: Option<String>,
    pub quota_used: Option<i64>,
    pub quota_total: Option<i64>,
    pub created_at: String,
}

/// A new account for [`insert`]; the tokens come from
/// `crypto::SecretKey::seal` with the account's ID.
pub struct NewAccount<'a> {
    pub id: &'a str,
    pub user_id: &'a str,
    pub base_url: &'a str,
    pub email: &'a str,
    pub account_id: &'a str,
    pub username: &'a str,
    pub auth_kind: AuthKind,
    pub client_id: Option<&'a str>,
    pub secret_enc: &'a [u8],
    pub access_token_enc: Option<&'a [u8]>,
    /// Validity of the access token in seconds from now.
    pub expires_in: Option<u32>,
    pub session_url: &'a str,
    pub session_json: &'a str,
    pub color: i64,
}

const COLUMNS: &str = "id, user_id, base_url, email, account_id, username, auth_kind, state, \
     color, is_default, session_url, session_json, last_check_at, quota_used, quota_total, \
     created_at";
/// Number of [`COLUMNS`].
const WIDTH: usize = 16;

fn conversion(i: usize, what: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        i,
        rusqlite::types::Type::Text,
        format!("unknown {what} {value}").into(),
    )
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<Account> {
    let kind: String = row.get(6)?;
    let state: String = row.get(7)?;
    Ok(Account {
        id: row.get(0)?,
        user_id: row.get(1)?,
        base_url: row.get(2)?,
        email: row.get(3)?,
        account_id: row.get(4)?,
        username: row.get(5)?,
        auth_kind: AuthKind::parse(&kind).ok_or_else(|| conversion(6, "sign-in kind", &kind))?,
        state: State::parse(&state).ok_or_else(|| conversion(7, "state", &state))?,
        color: row.get(8)?,
        is_default: row.get(9)?,
        session_url: row.get(10)?,
        session_json: row.get(11)?,
        last_check_at: row.get(12)?,
        quota_used: row.get(13)?,
        quota_total: row.get(14)?,
        created_at: row.get(15)?,
    })
}

/// A user's accounts in the order they were created.
pub fn list(conn: &Connection, user_id: &str) -> rusqlite::Result<Vec<Account>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM stalwart_accounts WHERE user_id = ?1 ORDER BY id"
    ))?;
    stmt.query_map([user_id], from_row)?.collect()
}

pub fn get(conn: &Connection, user_id: &str, id: &str) -> rusqlite::Result<Option<Account>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM stalwart_accounts WHERE id = ?1 AND user_id = ?2"),
        (id, user_id),
        from_row,
    )
    .optional()
}

/// A user's account with the server and account ID of the session (duplicate).
pub fn find(
    conn: &Connection,
    user_id: &str,
    base_url: &str,
    account_id: &str,
) -> rusqlite::Result<Option<Account>> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS} FROM stalwart_accounts \
             WHERE user_id = ?1 AND base_url = ?2 AND account_id = ?3"
        ),
        (user_id, base_url, account_id),
        from_row,
    )
    .optional()
}

/// Creates the account; the user's first one becomes the default account.
pub fn insert(conn: &Connection, account: &NewAccount<'_>) -> anyhow::Result<Account> {
    conn.query_row(
        &format!(
            "INSERT INTO stalwart_accounts (id, user_id, base_url, email, account_id, username, \
             auth_kind, client_id, secret_enc, access_token_enc, access_expires_at, \
             session_url, session_json, color, is_default) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, \
             CASE WHEN ?11 IS NULL THEN NULL \
                  ELSE strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+' || ?11 || ' seconds') END, \
             ?12, ?13, ?14, \
             NOT EXISTS (SELECT 1 FROM stalwart_accounts WHERE user_id = ?2)) \
             RETURNING {COLUMNS}"
        ),
        (
            account.id,
            account.user_id,
            account.base_url,
            account.email,
            account.account_id,
            account.username,
            account.auth_kind.as_str(),
            account.client_id,
            account.secret_enc,
            account.access_token_enc,
            account.expires_in,
            account.session_url,
            account.session_json,
            account.color,
        ),
        from_row,
    )
    .context("Stalwart account not created")
}

/// Tokens of an account for the token broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub account: Account,
    /// `oauth`: client the tokens were issued for.
    pub client_id: Option<String>,
    /// Refresh token or access token, encrypted.
    pub secret_enc: Vec<u8>,
    pub access_token_enc: Option<Vec<u8>>,
    /// RFC 3339 in UTC.
    pub access_expires_at: Option<String>,
    /// When a browser last fetched a token (RFC 3339 in UTC).
    pub token_used_at: Option<String>,
}

const CREDENTIALS: &str =
    "client_id, secret_enc, access_token_enc, access_expires_at, token_used_at";

fn credentials_from_row(row: &Row<'_>) -> rusqlite::Result<Credentials> {
    Ok(Credentials {
        account: from_row(row)?,
        client_id: row.get(WIDTH)?,
        secret_enc: row.get(WIDTH + 1)?,
        access_token_enc: row.get(WIDTH + 2)?,
        access_expires_at: row.get(WIDTH + 3)?,
        token_used_at: row.get(WIDTH + 4)?,
    })
}

/// Account with tokens; with `user_id` only an account of this user.
pub fn credentials(
    conn: &Connection,
    id: &str,
    user_id: Option<&str>,
) -> rusqlite::Result<Option<Credentials>> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS}, {CREDENTIALS} FROM stalwart_accounts \
             WHERE id = ?1 AND (?2 IS NULL OR user_id = ?2)"
        ),
        (id, user_id),
        credentials_from_row,
    )
    .optional()
}

/// All accounts of a user with tokens: their grants are revoked when the
/// user is deleted.
pub fn credentials_of_user(conn: &Connection, user_id: &str) -> rusqlite::Result<Vec<Credentials>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS}, {CREDENTIALS} FROM stalwart_accounts WHERE user_id = ?1 ORDER BY id"
    ))?;
    stmt.query_map([user_id], credentials_from_row)?.collect()
}

/// A browser just fetched an access token (active account).
pub fn touch_token_used(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE stalwart_accounts SET token_used_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE id = ?1",
        [id],
    )?;
    Ok(())
}

/// Tokens after a refresh, encrypted with the account's ID.
pub struct Refreshed<'a> {
    pub access_token_enc: &'a [u8],
    /// New refresh token (rotation); `None` keeps the previous one.
    pub secret_enc: Option<&'a [u8]>,
    /// Validity of the access token in seconds from now.
    pub expires_in: Option<u32>,
}

/// Stores access and refresh token in one step – only if the account still
/// has the refresh token `used`: the user may have signed it in again or
/// removed it meanwhile. `None` if not stored, otherwise the new expiry of
/// the access token.
pub fn store_refresh(
    conn: &Connection,
    id: &str,
    used: &[u8],
    refreshed: &Refreshed<'_>,
) -> rusqlite::Result<Option<Option<String>>> {
    conn.query_row(
        "UPDATE stalwart_accounts SET access_token_enc = ?3, \
         secret_enc = COALESCE(?4, secret_enc), \
         access_expires_at = CASE WHEN ?5 IS NULL THEN NULL \
              ELSE strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+' || ?5 || ' seconds') END \
         WHERE id = ?1 AND secret_enc = ?2 AND state = 'ok' \
         RETURNING access_expires_at",
        (
            id,
            used,
            refreshed.access_token_enc,
            refreshed.secret_enc,
            refreshed.expires_in,
        ),
        |row| row.get(0),
    )
    .optional()
}

/// The refresh with `used` failed for good: "Sign in again", the access
/// token is no longer valid. The user's ID if it changed; `None` if the
/// account has other tokens by now, is removed or was marked already.
pub fn mark_needs_sign_in(
    conn: &Connection,
    id: &str,
    used: &[u8],
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "UPDATE stalwart_accounts SET state = 'needs_sign_in', access_token_enc = NULL, \
         access_expires_at = NULL WHERE id = ?1 AND secret_enc = ?2 AND state = 'ok' \
         RETURNING user_id",
        (id, used),
        |row| row.get(0),
    )
    .optional()
}

/// New tokens after "Sign in again" ([`NewAccount`] without account details).
pub struct SignedIn<'a> {
    pub client_id: Option<&'a str>,
    pub secret_enc: &'a [u8],
    pub access_token_enc: Option<&'a [u8]>,
    pub expires_in: Option<u32>,
    pub session_url: &'a str,
    pub session_json: &'a str,
}

/// Replaces the tokens of one of the user's accounts; it is valid again
/// afterwards. The account as it was before, `None` if the user has none
/// with that ID.
pub fn signed_in_again(
    conn: &Connection,
    user_id: &str,
    id: &str,
    signed_in: &SignedIn<'_>,
) -> rusqlite::Result<Option<Account>> {
    let Some(before) = get(conn, user_id, id)? else {
        return Ok(None);
    };
    conn.execute(
        "UPDATE stalwart_accounts SET client_id = ?3, secret_enc = ?4, access_token_enc = ?5, \
         access_expires_at = CASE WHEN ?6 IS NULL THEN NULL \
              ELSE strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+' || ?6 || ' seconds') END, \
         session_url = ?7, session_json = ?8, state = 'ok' \
         WHERE id = ?1 AND user_id = ?2",
        (
            id,
            user_id,
            signed_in.client_id,
            signed_in.secret_enc,
            signed_in.access_token_enc,
            signed_in.expires_in,
            signed_in.session_url,
            signed_in.session_json,
        ),
    )?;
    Ok(Some(before))
}

/// Active accounts signed in at the server whose access token expires in
/// less than `margin` (or is missing): a browser fetched a token within
/// `active`, the account is still valid and its user is active – locked users
/// get no tokens anyway.
pub fn due_for_refresh(
    conn: &Connection,
    active: Duration,
    margin: Duration,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM stalwart_accounts \
         WHERE auth_kind = 'oauth' AND state = 'ok' \
           AND user_id IN (SELECT id FROM users WHERE status = 'active') \
           AND token_used_at >= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-' || ?1 || ' seconds') \
           AND (access_expires_at IS NULL \
                OR access_expires_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+' || ?2 || ' seconds')) \
         ORDER BY access_expires_at",
    )?;
    let secs = |d: Duration| i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
    stmt.query_map((secs(active), secs(margin)), |row| row.get(0))?
        .collect()
}

/// New colour of one of the user's accounts (index below [`COLORS`], checked
/// by the caller); `None` if the user has none with that ID.
pub fn set_color(
    conn: &Connection,
    user_id: &str,
    id: &str,
    color: i64,
) -> rusqlite::Result<Option<Account>> {
    conn.query_row(
        &format!(
            "UPDATE stalwart_accounts SET color = ?3 WHERE id = ?1 AND user_id = ?2 \
             RETURNING {COLUMNS}"
        ),
        (id, user_id, color),
        from_row,
    )
    .optional()
}

/// Makes one of the user's accounts the default account; `false` if the
/// user has none with that ID.
pub fn set_default(conn: &Connection, user_id: &str, id: &str) -> rusqlite::Result<bool> {
    crate::db::atomic(conn, |conn| {
        if !exists(conn, user_id, id)? {
            return Ok(false);
        }
        conn.execute(
            "UPDATE stalwart_accounts SET is_default = 0 \
             WHERE user_id = ?1 AND is_default = 1 AND id <> ?2",
            (user_id, id),
        )?;
        conn.execute(
            "UPDATE stalwart_accounts SET is_default = 1 WHERE id = ?1 AND user_id = ?2",
            (id, user_id),
        )?;
        Ok(true)
    })
}

/// Result of the account's check and job `quota_check`: storage in bytes,
/// used and limit (`None` without a limit), and `last_check_at` now; `None`
/// if the account has been removed meanwhile.
pub fn store_check(
    conn: &Connection,
    user_id: &str,
    id: &str,
    quota: Option<(u64, u64)>,
) -> rusqlite::Result<Option<Account>> {
    let bytes = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    let (used, total) = quota
        .map(|(used, total)| (bytes(used), bytes(total)))
        .unzip();
    conn.query_row(
        &format!(
            "UPDATE stalwart_accounts SET quota_used = ?3, quota_total = ?4, \
             last_check_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE id = ?1 AND user_id = ?2 RETURNING {COLUMNS}"
        ),
        (id, user_id, used, total),
        from_row,
    )
    .optional()
}

/// Accounts the job `quota_check` checks: signed in, the user active, the
/// job's last attempt longer than `every` ago or none yet; the ones not
/// tried for the longest first.
pub fn due_for_quota_check(conn: &Connection, every: Duration) -> rusqlite::Result<Vec<Account>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM stalwart_accounts \
         WHERE state = 'ok' \
           AND user_id IN (SELECT id FROM users WHERE status = 'active') \
           AND (quota_job_at IS NULL \
                OR quota_job_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-' || ?1 || ' seconds')) \
         ORDER BY quota_job_at IS NOT NULL, quota_job_at, id"
    ))?;
    let secs = i64::try_from(every.as_secs()).unwrap_or(i64::MAX);
    stmt.query_map([secs], from_row)?.collect()
}

/// Records an attempt of the job `quota_check`, also a failed one.
pub fn mark_quota_job(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE stalwart_accounts SET quota_job_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE id = ?1",
        [id],
    )?;
    Ok(())
}

/// Removes one of the user's accounts; if it was the default account, the
/// oldest remaining one becomes it. `false` if the user has none with that ID.
pub fn delete(conn: &Connection, user_id: &str, id: &str) -> rusqlite::Result<bool> {
    crate::db::atomic(conn, |conn| {
        let deleted = conn.execute(
            "DELETE FROM stalwart_accounts WHERE id = ?1 AND user_id = ?2",
            (id, user_id),
        )? == 1;
        if deleted {
            conn.execute(
                "UPDATE stalwart_accounts SET is_default = 1 \
                 WHERE id = (SELECT id FROM stalwart_accounts WHERE user_id = ?1 ORDER BY id \
                             LIMIT 1) \
                   AND NOT EXISTS (SELECT 1 FROM stalwart_accounts \
                                   WHERE user_id = ?1 AND is_default = 1)",
                [user_id],
            )?;
        }
        Ok(deleted)
    })
}

/// Whether the user has an account with the ID.
pub fn exists(conn: &Connection, user_id: &str, id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM stalwart_accounts WHERE id = ?1 AND user_id = ?2)",
        (id, user_id),
        |row| row.get(0),
    )
}

/// Accounts of all users per origin (linked accounts of the registry).
pub fn count_by_base_url(conn: &Connection) -> rusqlite::Result<HashMap<String, i64>> {
    let mut stmt =
        conn.prepare("SELECT base_url, COUNT(*) FROM stalwart_accounts GROUP BY base_url")?;
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

/// First colour none of the user's accounts has, otherwise the first.
pub fn free_color(accounts: &[Account]) -> i64 {
    (0..COLORS)
        .find(|color| accounts.iter().all(|account| account.color != *color))
        .unwrap_or(0)
}

/// A sign-in at a server under way, for [`insert_sign_in`].
pub struct NewSignIn<'a> {
    /// SHA-256 of `state` in hex.
    pub state_hash: &'a str,
    pub user_id: &'a str,
    pub session_id: &'a str,
    pub server_id: &'a str,
    pub base_url: &'a str,
    pub client_id: &'a str,
    /// "Sign in again" for this account.
    pub account_id: Option<&'a str>,
    pub verifier_enc: &'a [u8],
    pub login_hint: &'a str,
    pub return_to: &'a str,
    pub valid_for: Duration,
}

/// A sign-in as [`take_sign_in`] returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignIn {
    pub user_id: String,
    pub session_id: String,
    pub server_id: String,
    pub base_url: String,
    pub client_id: String,
    pub account_id: Option<String>,
    pub verifier_enc: Vec<u8>,
    pub login_hint: String,
    pub return_to: String,
    /// Expired by now (it is taken anyway, so that it counts once).
    pub expired: bool,
}

/// Records a sign-in; expired ones of every user go at the same time.
pub fn insert_sign_in(conn: &Connection, sign_in: &NewSignIn<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM stalwart_sign_ins \
         WHERE expires_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        [],
    )?;
    let secs = i64::try_from(sign_in.valid_for.as_secs()).unwrap_or(i64::MAX);
    conn.execute(
        "INSERT INTO stalwart_sign_ins (state_hash, user_id, session_id, server_id, base_url, \
         client_id, account_id, verifier_enc, login_hint, return_to, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, \
         strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+' || ?11 || ' seconds'))",
        (
            sign_in.state_hash,
            sign_in.user_id,
            sign_in.session_id,
            sign_in.server_id,
            sign_in.base_url,
            sign_in.client_id,
            sign_in.account_id,
            sign_in.verifier_enc,
            sign_in.login_hint,
            sign_in.return_to,
            secs,
        ),
    )?;
    Ok(())
}

/// Takes the sign-in with the state hash: it can be completed once.
pub fn take_sign_in(conn: &Connection, state_hash: &str) -> rusqlite::Result<Option<SignIn>> {
    conn.query_row(
        "DELETE FROM stalwart_sign_ins WHERE state_hash = ?1 \
         RETURNING user_id, session_id, server_id, base_url, client_id, account_id, \
         verifier_enc, login_hint, return_to, \
         expires_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        [state_hash],
        |row| {
            Ok(SignIn {
                user_id: row.get(0)?,
                session_id: row.get(1)?,
                server_id: row.get(2)?,
                base_url: row.get(3)?,
                client_id: row.get(4)?,
                account_id: row.get(5)?,
                verifier_enc: row.get(6)?,
                login_hint: row.get(7)?,
                return_to: row.get(8)?,
                expired: row.get(9)?,
            })
        },
    )
    .optional()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::repo::{
        stalwart_servers::tests::add as add_server,
        users::{
            Role,
            tests::{add, conn},
        },
    };

    pub(crate) fn account<'a>(
        id: &'a str,
        user_id: &'a str,
        account_id: &'a str,
    ) -> NewAccount<'a> {
        NewAccount {
            id,
            user_id,
            base_url: "https://mail.example.com",
            email: "max@example.com",
            account_id,
            username: "max@example.com",
            auth_kind: AuthKind::Oauth,
            client_id: Some("client"),
            secret_enc: b"sealed",
            access_token_enc: Some(b"sealed"),
            expires_in: Some(3600),
            session_url: "https://mail.example.com/jmap/session",
            session_json: "{}",
            color: 0,
        }
    }

    #[test]
    fn stores_accounts_per_user_with_one_default() {
        let conn = conn();
        let max = add(&conn, "max", "max@example.com", Role::User);
        let eva = add(&conn, "eva", "eva@example.com", Role::User);
        assert!(insert(&conn, &account("A1", &max, "a")).unwrap().is_default);
        // Another user may connect the same account too.
        assert!(insert(&conn, &account("A2", &eva, "a")).unwrap().is_default);
        assert!(insert(&conn, &account("A3", &max, "a")).is_err());
        assert!(!insert(&conn, &account("A4", &max, "b")).unwrap().is_default);
        assert_eq!(list(&conn, &max).unwrap().len(), 2);
        assert_eq!(
            find(&conn, &max, "https://mail.example.com", "b")
                .unwrap()
                .map(|a| a.id),
            Some("A4".to_owned())
        );
        assert!(!set_default(&conn, &eva, "A4").unwrap());
        assert!(set_default(&conn, &max, "A4").unwrap());
        let defaults: Vec<_> = list(&conn, &max)
            .unwrap()
            .into_iter()
            .map(|a| (a.id, a.is_default))
            .collect();
        assert_eq!(
            defaults,
            [("A1".to_owned(), false), ("A4".to_owned(), true)]
        );
        // Removing the default account makes the oldest remaining one it.
        assert!(!delete(&conn, &eva, "A4").unwrap());
        assert!(delete(&conn, &max, "A4").unwrap());
        assert!(get(&conn, &max, "A1").unwrap().unwrap().is_default);
        assert_eq!(
            count_by_base_url(&conn).unwrap()["https://mail.example.com"],
            2
        );
        assert_eq!(free_color(&list(&conn, &max).unwrap()), 1);
        assert_eq!(set_color(&conn, &max, "A1", 3).unwrap().unwrap().color, 3);
        assert_eq!(set_color(&conn, &eva, "A1", 3).unwrap(), None);
    }

    #[test]
    fn refresh_replaces_only_the_tokens_it_used() {
        let conn = conn();
        let max = add(&conn, "max", "max@example.com", Role::User);
        insert(&conn, &account("A1", &max, "a")).unwrap();
        let (hour, soon) = (Duration::from_secs(3600), Duration::from_secs(7200));
        // Not used by a browser yet.
        assert!(due_for_refresh(&conn, hour, soon).unwrap().is_empty());
        touch_token_used(&conn, "A1").unwrap();
        assert_eq!(due_for_refresh(&conn, hour, soon).unwrap(), ["A1"]);
        let refreshed = Refreshed {
            access_token_enc: b"new",
            secret_enc: Some(b"rotated"),
            expires_in: Some(60),
        };
        assert_eq!(
            store_refresh(&conn, "A1", b"other", &refreshed).unwrap(),
            None
        );
        let expires = store_refresh(&conn, "A1", b"sealed", &refreshed)
            .unwrap()
            .unwrap()
            .unwrap();
        let creds = credentials(&conn, "A1", Some(&max)).unwrap().unwrap();
        assert_eq!(creds.secret_enc, b"rotated");
        assert_eq!(creds.access_expires_at, Some(expires));
        assert!(credentials(&conn, "A1", Some("eva")).unwrap().is_none());
        assert_eq!(credentials_of_user(&conn, &max).unwrap().len(), 1);
        assert_eq!(mark_needs_sign_in(&conn, "A1", b"sealed").unwrap(), None);
        assert_eq!(
            mark_needs_sign_in(&conn, "A1", b"rotated").unwrap(),
            Some(max.clone())
        );
        assert_eq!(mark_needs_sign_in(&conn, "A1", b"rotated").unwrap(), None);
        let creds = credentials(&conn, "A1", None).unwrap().unwrap();
        assert_eq!(creds.account.state, State::NeedsSignIn);
        assert_eq!(creds.access_token_enc, None);
        assert!(due_for_refresh(&conn, hour, soon).unwrap().is_empty());
        assert_eq!(
            store_refresh(&conn, "A1", b"rotated", &refreshed).unwrap(),
            None
        );
        let again = SignedIn {
            client_id: Some("client"),
            secret_enc: b"fresh",
            access_token_enc: Some(b"access"),
            expires_in: Some(3600),
            session_url: "https://mail.example.com/jmap/session",
            session_json: "{}",
        };
        let before = signed_in_again(&conn, &max, "A1", &again).unwrap().unwrap();
        assert_eq!(before.state, State::NeedsSignIn);
        assert_eq!(get(&conn, &max, "A1").unwrap().unwrap().state, State::Ok);
        assert_eq!(signed_in_again(&conn, "eva", "A1", &again).unwrap(), None);
    }

    #[test]
    fn quota_job_checks_each_account_once_a_day() {
        let conn = conn();
        let max = add(&conn, "max", "max@example.com", Role::User);
        insert(&conn, &account("A1", &max, "a")).unwrap();
        insert(&conn, &account("A2", &max, "b")).unwrap();
        let day = Duration::from_secs(86_400);
        let due = |conn: &Connection| -> Vec<String> {
            due_for_quota_check(conn, day)
                .unwrap()
                .into_iter()
                .map(|a| a.id)
                .collect()
        };
        assert_eq!(due(&conn), ["A1", "A2"]);
        mark_quota_job(&conn, "A1").unwrap();
        assert_eq!(due(&conn), ["A2"]);
        let checked = store_check(&conn, &max, "A2", Some((1, 2)))
            .unwrap()
            .unwrap();
        assert_eq!(
            (checked.quota_used, checked.quota_total),
            (Some(1), Some(2))
        );
        let unlimited = store_check(&conn, &max, "A2", None).unwrap().unwrap();
        assert_eq!((unlimited.quota_used, unlimited.quota_total), (None, None));
        assert_eq!(store_check(&conn, "eva", "A2", None).unwrap(), None);
    }

    #[test]
    fn sign_ins_are_taken_once() {
        let conn = conn();
        let max = add(&conn, "max", "max@example.com", Role::User);
        add_server(&conn, "s1", "https://mail.s1.test");
        let session = crate::auth::session::start(
            &conn,
            &max,
            &crate::auth::session::Client::default(),
            true,
        )
        .unwrap()
        .id;
        let new = |hash, valid_for| NewSignIn {
            state_hash: hash,
            user_id: &max,
            session_id: &session,
            server_id: "s1",
            base_url: "https://mail.s1.test",
            client_id: "roedy-hub",
            account_id: None,
            verifier_enc: b"sealed",
            login_hint: "max@example.com",
            return_to: "/mail/",
            valid_for,
        };
        insert_sign_in(&conn, &new("h1", Duration::from_secs(600))).unwrap();
        insert_sign_in(&conn, &new("h2", Duration::ZERO)).unwrap();
        let taken = take_sign_in(&conn, "h1").unwrap().unwrap();
        assert_eq!(taken.return_to, "/mail/");
        assert!(!taken.expired);
        assert_eq!(take_sign_in(&conn, "h1").unwrap(), None);
        // Expired: still taken, so that it counts once.
        std::thread::sleep(Duration::from_millis(5));
        assert!(take_sign_in(&conn, "h2").unwrap().unwrap().expired);
    }
}
