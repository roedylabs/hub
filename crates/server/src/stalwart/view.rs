//! A user's Stalwart account as the apps see it (contract § 6.1): the browser
//! API, the internal API and the `stalwart.account.added` event share it. No
//! tokens, no session document – an app reads the session with an access
//! token itself.

use std::collections::{BTreeSet, HashMap};

use rusqlite::Connection;
use serde::Serialize;

use super::blocked;
use crate::db::repo::{
    stalwart_accounts::{self as accounts, Account, State},
    stalwart_servers::{self as servers, Server},
};

/// The registry server of an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerRef {
    pub id: String,
    pub label: String,
}

/// Storage as of the last check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Quota {
    /// Bytes; `null` without a limit.
    pub used: Option<i64>,
    pub total: Option<i64>,
    pub checked_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountView {
    pub id: String,
    pub email: String,
    /// `username` of the JMAP session.
    pub username: String,
    /// Origin of the server.
    pub url: String,
    /// The registry server with this origin; `null` for a custom server.
    pub server: Option<ServerRef>,
    /// `oauth` (signed in at the server) or `token`.
    pub auth: &'static str,
    pub session_url: String,
    /// The capabilities of the session at the last sign-in, sorted.
    pub capabilities: Vec<String>,
    pub default: bool,
    /// Index into `MA_COLORS` of `@roedy/tokens`.
    pub color: i64,
    /// `ok`, `needs_sign_in` or `blocked` (custom server while the policy
    /// is off: no tokens until it is on again or the server is in the
    /// registry).
    pub state: &'static str,
    pub quota: Option<Quota>,
    pub created_at: String,
}

/// The capabilities named in a session document.
fn capabilities(session_json: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(session_json)
        .ok()
        .and_then(|session| {
            session
                .get("capabilities")?
                .as_object()
                .map(|caps| caps.keys().cloned().collect::<BTreeSet<_>>())
        })
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// The view of `account` with its registry server (`None`: custom server)
/// and the policy.
pub fn view(account: Account, server: Option<&Server>, allow_custom: bool) -> AccountView {
    let state = if blocked(&account.base_url, server.is_some(), allow_custom) {
        "blocked"
    } else {
        account.state.as_str()
    };
    AccountView {
        capabilities: capabilities(&account.session_json),
        server: server.map(|server| ServerRef {
            id: server.id.clone(),
            label: server.label.clone(),
        }),
        quota: account.last_check_at.map(|checked_at| Quota {
            used: account.quota_used,
            total: account.quota_total,
            checked_at,
        }),
        id: account.id,
        email: account.email,
        username: account.username,
        url: account.base_url,
        auth: account.auth_kind.as_str(),
        session_url: account.session_url,
        default: account.is_default,
        color: account.color,
        state,
        created_at: account.created_at,
    }
}

/// The view of one account with the registry and the policy as of now.
pub fn current(conn: &Connection, account: Account) -> anyhow::Result<AccountView> {
    let server = servers::find_by_base_url(conn, &account.base_url)?;
    let allow = servers::allow_custom_servers(conn)?;
    Ok(view(account, server.as_ref(), allow))
}

/// A user's accounts in the order they were created.
pub fn of_user(conn: &Connection, user_id: &str) -> anyhow::Result<Vec<AccountView>> {
    let registry: HashMap<String, Server> = servers::list(conn)?
        .into_iter()
        .map(|server| (server.base_url.clone(), server))
        .collect();
    let allow = servers::allow_custom_servers(conn)?;
    Ok(accounts::list(conn, user_id)?
        .into_iter()
        .map(|account| {
            let server = registry.get(&account.base_url);
            view(account, server, allow)
        })
        .collect())
}

impl AccountView {
    /// Whether the account can hand out tokens now.
    pub fn usable(&self) -> bool {
        self.state == State::Ok.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_come_sorted_from_the_session() {
        let session = r#"{"capabilities": {"urn:ietf:params:jmap:mail": {},
            "urn:ietf:params:jmap:core": {}}}"#;
        assert_eq!(
            capabilities(session),
            ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"]
        );
        assert!(capabilities("{}").is_empty());
        assert!(capabilities("[]").is_empty());
    }
}
