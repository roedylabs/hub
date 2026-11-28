//! Token broker: for a Stalwart account a browser (or an app's backend)
//! fetches a short-lived access token and uses it to talk JMAP directly. The
//! hub keeps the refresh token and renews the access token before it
//! expires – at most one refresh per account at a time ([`KeyedLocks`]), new
//! tokens stored in one step (rotation). Stalwart v1 rotates strictly: an
//! access token only goes out once the new refresh token is stored, and the
//! old one goes to the server only once more if the new one was never stored
//! (response lost) – otherwise Stalwart would revoke the whole grant. If the
//! server rejects the refresh token or the client, the account needs a new
//! sign-in (`needs_sign_in`, event `stalwart.account.needs_sign_in`);
//! temporary errors change nothing, and a still valid access token keeps
//! going out. After a temporary error the broker does not ask the server
//! again for a while ([`REFRESH_FAILED_LIMIT`]). The hub exchanged every code
//! itself, so it refreshes with the registry client's secret where there is
//! one. When an account is removed it revokes the grant ([`revoke`]), as well
//! as a grant from a code exchange that it does not keep
//! ([`revoke_discarded`]).
//!
//! Taken over from Roedy Mail (`crates/server/src/broker/token.rs`) without
//! the grants from the browser (`backend_exchange`), which Mail refreshed as
//! a public client.
//!
//! [`KeyedLocks`]: crate::locks::KeyedLocks

use std::time::Duration;

use anyhow::Context as _;
use jiff::{SignedDuration, Timestamp};
use url::Url;

use super::{blocked, oauth};
use crate::{
    app::AppState,
    db::{
        self,
        repo::{
            events::{self, Kind},
            stalwart_accounts::{
                self as accounts, ACCESS_PURPOSE, AuthKind, Credentials, Refreshed, SECRET_PURPOSE,
                State,
            },
            stalwart_servers::{self as servers, SECRET_PURPOSE as CLIENT_SECRET_PURPOSE, Server},
        },
    },
    locks::KeyedLock,
    rate_limit::{Key, Limited, Policy},
};

/// If the access token is valid for less, the broker renews it before
/// handing it out.
pub const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);
/// The early refresh renews active accounts before that, so that the browser
/// does not wait for the server.
pub const PREFETCH_MARGIN: Duration = Duration::from_secs(10 * 60);
/// An account is active if a browser fetched a token for it within this
/// time.
pub const ACTIVE: Duration = Duration::from_secs(60 * 60);
/// The precision with which the hub records when a browser last fetched a
/// token – not a write on every fetch.
const TOUCH_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Rejected clients on refresh per server: a wrong secret fails from the
/// hub's address, and Stalwart locks addresses after failed sign-ins
/// (Fail2Ban) – then for all accounts on the server.
const CLIENT_REJECTED_LIMIT: Policy = Policy::lockout(
    "stalwart.refresh_rejected",
    5,
    Duration::from_secs(15 * 60),
    Duration::from_secs(15 * 60),
);

/// Pause per server after a temporary error (not reachable, 5xx, 429 …):
/// waiting calls, other accounts and the early refresh do not ask again
/// during that time and get the previous access token or 429 with
/// `Retry-After`.
pub const REFRESH_FAILED_LIMIT: Policy = Policy::lockout(
    "stalwart.refresh_failed",
    1,
    Duration::from_secs(30),
    Duration::from_secs(30),
);

/// What the browser or the app's backend gets.
#[derive(Debug)]
pub struct Access {
    pub access_token: String,
    /// RFC 3339 in UTC; `None` for access tokens the user entered (`token`).
    pub expires_at: Option<String>,
}

#[derive(Debug)]
pub enum Error {
    /// No account with the ID (for the user).
    NotFound,
    /// Custom server while the policy is turned off.
    Blocked,
    /// The account needs a new sign-in.
    NeedsSignIn,
    /// Refresh failed temporarily, and no access token is valid any more.
    Refresh(oauth::Error),
    /// Refresh for the server throttled ([`CLIENT_REJECTED_LIMIT`],
    /// [`REFRESH_FAILED_LIMIT`]), and no access token is valid any more.
    Limited(Limited),
    Internal(anyhow::Error),
}

impl Error {
    /// Short reason for the log, without details of the server.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Blocked => "blocked",
            Self::NeedsSignIn => "needs_sign_in",
            Self::Refresh(error) => error.reason(),
            Self::Limited(_) => "limited",
            Self::Internal(_) => "internal",
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}

/// Access token for the browser of `user_id`. `force` renews it although it
/// is still valid: the server rejected it (e.g. after a new password). If
/// another call renewed it meanwhile, that call's token applies.
pub async fn access(
    state: &AppState,
    user_id: &str,
    id: &str,
    force: bool,
) -> Result<Access, Error> {
    fetch(state, id, Some(user_id), true, force).await
}

/// Access token for a job (`quota_check`) or an app's backend: like
/// [`access`], but without counting as a browser fetch – otherwise the early
/// refresh would consider every such account active for an hour. With
/// `user_id` only an account of that user.
pub async fn for_backend(
    state: &AppState,
    id: &str,
    user_id: Option<&str>,
    force: bool,
) -> Result<Access, Error> {
    fetch(state, id, user_id, false, force).await
}

/// [`access`] and [`for_backend`]: with `touch` a browser asks (active
/// account, [`ACTIVE`]).
async fn fetch(
    state: &AppState,
    id: &str,
    user_id: Option<&str>,
    touch: bool,
    force: bool,
) -> Result<Access, Error> {
    let loaded = load(state, id, user_id, touch).await?;
    if loaded.creds.account.auth_kind == AuthKind::Token {
        let token = open(state, SECRET_PURPOSE, &loaded.creds.secret_enc, id)?;
        return Ok(Access {
            access_token: token,
            expires_at: None,
        });
    }
    if !force && let Some(cached) = cached(state, &loaded.creds, REFRESH_MARGIN)? {
        return Ok(cached);
    }
    let seen = loaded.creds.access_token_enc;
    let lock = state.stalwart.token_locks.lock(id).await;
    let loaded = load(state, id, user_id, false).await?;
    let renewed = loaded.creds.access_token_enc != seen;
    if !force || renewed {
        let margin = if force {
            Duration::ZERO
        } else {
            REFRESH_MARGIN
        };
        if let Some(cached) = cached(state, &loaded.creds, margin)? {
            return Ok(cached);
        }
    }
    spawn_refresh(state, loaded, lock).await
}

/// Early refresh of an active account (job `token_refresh`): only if its
/// access token expires in less than [`PREFETCH_MARGIN`].
pub async fn prefetch(state: &AppState, id: &str) -> Result<(), Error> {
    let lock = state.stalwart.token_locks.lock(id).await;
    let loaded = load(state, id, None, false).await?;
    if loaded.creds.account.auth_kind == AuthKind::Token
        || valid_for(&loaded.creds, PREFETCH_MARGIN)
    {
        return Ok(());
    }
    spawn_refresh(state, loaded, lock).await.map(drop)
}

/// Decrypted client secret of a registry server for `client_id`.
/// The secret belongs only to the registry's client; another client (e.g.
/// after a new registration) goes without it, as does a `server` whose
/// address or client has changed in the registry since it was read.
pub async fn client_secret(
    state: &AppState,
    server: &Server,
    client_id: &str,
) -> anyhow::Result<Option<String>> {
    if !server.has_secret || server.client_id.as_deref() != Some(client_id) {
        return Ok(None);
    }
    let (id, base_url, client_id) = (
        server.id.clone(),
        server.base_url.clone(),
        client_id.to_owned(),
    );
    let Some(sealed) = db::call(&state.db, move |conn| {
        Ok(servers::client_secret(conn, &id, &base_url, &client_id)?)
    })
    .await?
    else {
        return Ok(None);
    };
    open(state, CLIENT_SECRET_PURPOSE, &sealed, &server.id).map(Some)
}

/// Revokes the grant of a just removed account at the server in its own
/// task: the removal does not wait for it, and errors change nothing about
/// it, only the log names them. Only for accounts signed in at the server
/// (`oauth`) that were last in order – a rejected refresh token is no longer
/// valid anyway –, and not while the server throttles rejected clients
/// ([`CLIENT_REJECTED_LIMIT`]). Blocked accounts the caller does not hand
/// over in the first place.
pub fn revoke(state: &AppState, creds: Credentials, server: Option<Server>) {
    if creds.account.auth_kind != AuthKind::Oauth || creds.account.state != State::Ok {
        return;
    }
    let task = state.clone();
    state.background.spawn(async move {
        let state = task;
        let result = async {
            let as_client = AsClient::of(&state, &creds, server.as_ref()).await?;
            revoke_as(&state, &creds.account.base_url, &as_client).await
        }
        .await;
        let id = creds.account.id.as_str();
        let origin = &creds.account.base_url;
        match result {
            Ok(Ok(())) => {
                tracing::info!(account = %id, url = %origin, "grant revoked at the Stalwart server");
            }
            Ok(Err(reason)) => {
                tracing::warn!(account = %id, url = %origin, reason, "grant not revoked at the Stalwart server");
            }
            Err(error) => {
                tracing::warn!(account = %id, url = %origin, "grant not revoked at the Stalwart server: {error:#}");
            }
        }
    });
}

/// A grant the hub just obtained by exchanging a code at a registry server,
/// but does not keep: its session is unusable, the account is connected
/// already (409 `dup`), the session names another account (409
/// `other_account`) or the account was removed while signing in again.
/// Nobody else holds its refresh token.
pub struct Discarded {
    pub server: Server,
    pub client_id: String,
    /// The client secret the exchange used, if any: the revocation uses the
    /// same, even if the administrator removed or changed the server since.
    pub client_secret: Option<String>,
    pub refresh_token: String,
}

/// Revokes a [`Discarded`] grant at the server in its own task, like
/// [`revoke`]: the response does not wait for it, and errors only appear in
/// the log. Otherwise the grant would stay at the server until it expires
/// from inactivity.
pub fn revoke_discarded(state: &AppState, user_id: &str, discarded: Discarded) {
    let (task, user_id) = (state.clone(), user_id.to_owned());
    state.background.spawn(async move {
        let state = task;
        let Discarded {
            server,
            client_id,
            client_secret,
            refresh_token,
        } = discarded;
        let result = async {
            // The address and the secret of the exchange: the grant belongs
            // to them, whatever the registry says now.
            let as_client = AsClient {
                http: &state.stalwart.http,
                secret: client_secret,
                client_id: &client_id,
                refresh_token,
            };
            revoke_as(&state, &server.base_url, &as_client).await
        }
        .await;
        let origin = &server.base_url;
        match result {
            Ok(Ok(())) => {
                tracing::info!(user = %user_id, url = %origin, "discarded grant revoked at the Stalwart server");
            }
            Ok(Err(reason)) => {
                tracing::warn!(user = %user_id, url = %origin, reason, "discarded grant not revoked at the Stalwart server");
            }
            Err(error) => {
                tracing::warn!(user = %user_id, url = %origin, "discarded grant not revoked at the Stalwart server: {error:#}");
            }
        }
    });
}

/// The revocation itself; `Ok(Err(reason))` if the server was not asked or
/// refused.
async fn revoke_as(
    state: &AppState,
    base_url: &str,
    as_client: &AsClient<'_>,
) -> anyhow::Result<Result<(), &'static str>> {
    let base_url = Url::parse(base_url).context("base_url invalid")?;
    let key = Key::Server(base_url.origin().ascii_serialization());
    if state
        .limiter
        .remaining(&CLIENT_REJECTED_LIMIT, &key)
        .is_err()
    {
        return Ok(Err("limited"));
    }
    let result = oauth::revoke(
        as_client.http,
        &base_url,
        as_client.client(),
        &as_client.refresh_token,
    )
    .await;
    if let Err(error) = &result {
        count_rejected_client(state, error, as_client, key);
    }
    Ok(result.map_err(|error| error.reason()))
}

/// How the hub appears at an account's server: HTTP client (registry server
/// or custom server with SSRF guard), the account's client with the secret –
/// only for the registry's client ([`client_secret`]) – and the refresh
/// token.
struct AsClient<'a> {
    http: &'a reqwest::Client,
    client_id: &'a str,
    secret: Option<String>,
    refresh_token: String,
}

impl<'a> AsClient<'a> {
    async fn of(
        state: &'a AppState,
        creds: &'a Credentials,
        server: Option<&Server>,
    ) -> anyhow::Result<Self> {
        let client_id = creds
            .client_id
            .as_deref()
            .context("Stalwart account without OAuth client")?;
        let secret = match server {
            Some(server) => client_secret(state, server, client_id).await?,
            None => None,
        };
        let http = state.stalwart.http(server.is_some());
        let refresh_token = open(state, SECRET_PURPOSE, &creds.secret_enc, &creds.account.id)?;
        Ok(Self {
            http,
            client_id,
            secret,
            refresh_token,
        })
    }

    fn client(&self) -> oauth::Client<'_> {
        oauth::Client {
            id: self.client_id,
            secret: self.secret.as_deref(),
        }
    }
}

/// Counts a rejected client against [`CLIENT_REJECTED_LIMIT`] – only if the
/// hub sent a secret: only a wrong secret counts for Fail2Ban at Stalwart; an
/// old or unknown client without a secret (e.g. after a new registration)
/// would otherwise block the refreshes of all accounts of the server. If the
/// server is already blocked, it stays so.
fn count_rejected_client(
    state: &AppState,
    error: &oauth::Error,
    as_client: &AsClient<'_>,
    key: Key,
) {
    if *error == oauth::Error::InvalidClient && as_client.secret.is_some() {
        let _ = state.limiter.acquire(&CLIENT_REJECTED_LIMIT, key);
    }
}

/// Account with tokens and the registry server with its origin.
struct Loaded {
    creds: Credentials,
    server: Option<Server>,
}

/// Loads an account that may get tokens. `touch`: a browser asks (active
/// account, [`ACTIVE`]).
async fn load(
    state: &AppState,
    id: &str,
    user_id: Option<&str>,
    touch: bool,
) -> Result<Loaded, Error> {
    let (id, user_id) = (id.to_owned(), user_id.map(str::to_owned));
    let found = db::call(&state.db, move |conn| {
        let Some(creds) = accounts::credentials(conn, &id, user_id.as_deref())? else {
            return Ok(None);
        };
        let server = servers::find_by_base_url(conn, &creds.account.base_url)?;
        let allow = servers::allow_custom_servers(conn)?;
        let blocked = blocked(&creds.account.base_url, server.is_some(), allow);
        if touch && !blocked && creds.account.state == State::Ok && !used_lately(&creds) {
            accounts::touch_token_used(conn, &id)?;
        }
        Ok(Some((Loaded { creds, server }, blocked)))
    })
    .await?;
    match found {
        None => Err(Error::NotFound),
        Some((_, true)) => Err(Error::Blocked),
        Some((loaded, _)) if loaded.creds.account.state != State::Ok => Err(Error::NeedsSignIn),
        Some((loaded, false)) => Ok(loaded),
    }
}

/// Did a browser fetch the token less than [`TOUCH_INTERVAL`] ago?
fn used_lately(creds: &Credentials) -> bool {
    let interval = SignedDuration::try_from(TOUCH_INTERVAL).unwrap_or(SignedDuration::MAX);
    creds
        .token_used_at
        .as_deref()
        .and_then(|at| at.parse::<Timestamp>().ok())
        .is_some_and(|at| Timestamp::now().duration_since(at) < interval)
}

/// The refresh runs to completion in its own task, even if the caller
/// aborts (the browser its request, the job on shutdown): a rotated refresh
/// token would otherwise be lost.
async fn spawn_refresh(state: &AppState, loaded: Loaded, lock: KeyedLock) -> Result<Access, Error> {
    let state = state.clone();
    tokio::spawn(async move { refresh(&state, loaded, lock).await })
        .await
        .context("refresh aborted")?
}

/// Renews the access token with the refresh token; `_lock` holds up other
/// refreshes of the account until the new tokens are stored. If the account
/// got new tokens meanwhile (signed in again), their access token or a
/// refresh with their refresh token applies.
async fn refresh(state: &AppState, mut loaded: Loaded, _lock: KeyedLock) -> Result<Access, Error> {
    let id = loaded.creds.account.id.clone();
    for _ in 0..3 {
        if let Some(access) = refresh_once(state, loaded).await? {
            return Ok(access);
        }
        loaded = load(state, &id, None, false).await?;
        if let Some(cached) = cached(state, &loaded.creds, Duration::ZERO)? {
            return Ok(cached);
        }
    }
    Err(anyhow::anyhow!("Stalwart account changed on every refresh").into())
}

/// One refresh with the tokens from `loaded`; `None` if the account has
/// other ones meanwhile.
async fn refresh_once(state: &AppState, loaded: Loaded) -> Result<Option<Access>, Error> {
    let Loaded { creds, server } = loaded;
    let id = creds.account.id.as_str();
    let base_url = Url::parse(&creds.account.base_url).context("base_url invalid")?;
    let origin = base_url.origin().ascii_serialization();
    let as_client = AsClient::of(state, &creds, server.as_ref()).await?;

    // Only check, do not reserve: concurrent refreshes of different
    // accounts are not a rejected client.
    let key = Key::Server(origin.clone());
    for policy in [&CLIENT_REJECTED_LIMIT, &REFRESH_FAILED_LIMIT] {
        if let Err(limited) = state.limiter.remaining(policy, &key) {
            return fallback(state, &creds, Error::Limited(limited)).map(Some);
        }
    }
    let result = oauth::request(
        as_client.http,
        &base_url,
        as_client.client(),
        oauth::Grant::Refresh(&as_client.refresh_token),
    )
    .await;

    match result {
        Ok(tokens) => store(state, &creds, tokens).await,
        Err(error) if error.is_rejected() => {
            count_rejected_client(state, &error, &as_client, key);
            tracing::warn!(account = %id, url = %origin, reason = error.reason(), "refresh rejected: sign-in needed");
            let (row, used) = (id.to_owned(), creds.secret_enc.clone());
            let marked = db::call(&state.db, move |conn| {
                db::atomic(conn, |conn| {
                    let Some(user_id) = accounts::mark_needs_sign_in(conn, &row, &used)? else {
                        return Ok(false);
                    };
                    events::stalwart_account(
                        conn,
                        Kind::StalwartAccountNeedsSignIn,
                        &user_id,
                        &row,
                    )?;
                    Ok(true)
                })
            })
            .await?;
            if marked {
                Err(Error::NeedsSignIn)
            } else {
                Ok(None)
            }
        }
        Err(error) => {
            let _ = state.limiter.acquire(&REFRESH_FAILED_LIMIT, key);
            tracing::warn!(account = %id, url = %origin, reason = error.reason(), "refresh failed");
            fallback(state, &creds, Error::Refresh(error)).map(Some)
        }
    }
}

/// Stores the new tokens – only if the account still has the used refresh
/// token (otherwise `None`).
async fn store(
    state: &AppState,
    creds: &Credentials,
    tokens: oauth::Tokens,
) -> Result<Option<Access>, Error> {
    let id = creds.account.id.clone();
    let access_enc = state
        .secrets
        .seal(ACCESS_PURPOSE, &id, tokens.access_token.as_bytes())?;
    let secret_enc = tokens
        .refresh_token
        .as_ref()
        .map(|token| state.secrets.seal(SECRET_PURPOSE, &id, token.as_bytes()))
        .transpose()?;
    let (row, used, expires_in) = (id.clone(), creds.secret_enc.clone(), tokens.expires_in);
    let stored = db::call(&state.db, move |conn| {
        Ok(accounts::store_refresh(
            conn,
            &row,
            &used,
            &Refreshed {
                access_token_enc: &access_enc,
                secret_enc: secret_enc.as_deref(),
                expires_in,
            },
        )?)
    })
    .await?;
    match stored {
        Some(expires_at) => {
            tracing::debug!(account = %id, rotated = tokens.refresh_token.is_some(), "access token renewed");
            Ok(Some(Access {
                access_token: tokens.access_token,
                expires_at,
            }))
        }
        // Signed in again or removed meanwhile.
        None => Ok(None),
    }
}

/// After a temporary error: the previous access token while it is valid.
fn fallback(state: &AppState, creds: &Credentials, error: Error) -> Result<Access, Error> {
    cached(state, creds, Duration::ZERO)?.ok_or(error)
}

/// The stored access token if it is valid for longer than `margin`.
fn cached(
    state: &AppState,
    creds: &Credentials,
    margin: Duration,
) -> Result<Option<Access>, Error> {
    let Some(sealed) = &creds.access_token_enc else {
        return Ok(None);
    };
    if !valid_for(creds, margin) {
        return Ok(None);
    }
    let token = open(state, ACCESS_PURPOSE, sealed, &creds.account.id)?;
    Ok(Some(Access {
        access_token: token,
        expires_at: creds.access_expires_at.clone(),
    }))
}

/// Is the access token valid for longer than `margin`? Without an expiry it
/// is not: without `expires_in` there is no telling when the server discards
/// it.
fn valid_for(creds: &Credentials, margin: Duration) -> bool {
    let margin = SignedDuration::try_from(margin).unwrap_or(SignedDuration::MAX);
    creds.access_token_enc.is_some()
        && creds
            .access_expires_at
            .as_deref()
            .and_then(|at| at.parse::<Timestamp>().ok())
            .is_some_and(|at| at.duration_since(Timestamp::now()) > margin)
}

fn open(state: &AppState, purpose: &str, sealed: &[u8], row: &str) -> anyhow::Result<String> {
    let plain = state.secrets.open(purpose, row, sealed)?;
    String::from_utf8(plain).context("token is not UTF-8")
}
