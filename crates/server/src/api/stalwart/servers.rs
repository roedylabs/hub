//! Stalwart server registry (Administration › Stalwart servers).
//!
//! - `GET stalwart/servers`: for every signed-in user – servers without the
//!   secret and the policy, for connecting accounts.
//! - `GET/POST admin/stalwart/servers`, `GET/PATCH/DELETE
//!   admin/stalwart/servers/:id`, `GET/PUT admin/stalwart/policy`:
//!   maintenance by administrators; the list also names the redirect URI and
//!   the system mail's server, each server the number of linked accounts
//!   (`accounts`, accounts of all users with its origin).
//! - `POST admin/stalwart/servers/:id/test`: diagnosis ([`probe`]).
//! - `POST admin/stalwart/servers/:id/register-client`: the hub's OAuth
//!   client via DCR ([`dcr`]) with an administrator's credentials at the
//!   server; client ID and secret go straight into the saved server.
//!
//! Every change ends up in the audit log; a new, moved or removed origin and
//! a change of the policy give `stalwart.servers.changed` to the apps, whose
//! CSP names the origins.
//!
//! Taken over from Roedy Mail (`crates/server/src/api/servers.rs`). Unlike
//! Mail the client is registered for a saved server, so the secret needs no
//! pending registration.

use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::{
    api::{ApiError, FieldErrors, JsonBody, error::retry_secs, nullable},
    app::AppState,
    auth::{AdminUser, AuthUser, CurrentUser},
    client_ip::ClientIp,
    db::{
        self,
        repo::{
            audit_log::{self, Category, Entry},
            events, stalwart_accounts as accounts,
            stalwart_servers::{self as servers, Fields, SECRET_PURPOSE, SecretChange, Server},
        },
    },
    mailer::system_mail,
    rate_limit::{Key, Policy},
    stalwart::{
        dcr, normalize_url,
        probe::{self, Reasons, Status},
        redirect_uri,
    },
};

pub(super) const MAX_LABEL: usize = 100;
const MAX_DOMAINS: usize = 100;
const MAX_DOMAIN: usize = 253;
const MAX_CLIENT_FIELD: usize = 1024;

/// Failed registrations: Stalwart blocks addresses after failed sign-ins
/// (Fail2Ban) – here that would be the hub's, and with it later the token
/// refresh of all users.
const REGISTER_LIMIT: Policy = Policy::lockout(
    "stalwart.register",
    5,
    Duration::from_secs(15 * 60),
    Duration::from_secs(15 * 60),
);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/stalwart/servers", get(list))
        .route("/admin/stalwart/servers", get(admin_list).post(create))
        .route("/admin/stalwart/policy", get(policy).put(set_policy))
        .route(
            "/admin/stalwart/servers/{id}",
            get(admin_get).patch(update).delete(remove),
        )
        .route("/admin/stalwart/servers/{id}/test", post(test))
        .route(
            "/admin/stalwart/servers/{id}/register-client",
            post(register_client),
        )
}

/// Server as every signed-in user sees it.
#[derive(Serialize)]
struct PublicServer {
    id: String,
    label: String,
    url: String,
    host: String,
    domains: Vec<String>,
    /// The hub's OAuth client is set up: users sign in at the server
    /// (`POST stalwart/sign-in`); otherwise only with an access token.
    sign_in: bool,
    reach: Status,
    cors: Status,
    oauth: Status,
}

/// Server for the administration; the secret only as "present".
#[derive(Serialize)]
struct AdminServer {
    id: String,
    label: String,
    url: String,
    host: String,
    domains: Vec<String>,
    client_id: Option<String>,
    has_secret: bool,
    reach: Status,
    cors: Status,
    oauth: Status,
    tested_at: Option<String>,
    /// Number of linked accounts.
    accounts: i64,
    created_at: String,
}

/// Host with port as the list shows it (`mail.example.com:8443`).
pub(super) fn host_of(base_url: &str) -> String {
    Url::parse(base_url)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?.to_owned();
            Some(match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            })
        })
        .unwrap_or_default()
}

impl From<Server> for PublicServer {
    fn from(s: Server) -> Self {
        Self {
            host: host_of(&s.base_url),
            sign_in: s.client_id.is_some(),
            id: s.id,
            label: s.label,
            url: s.base_url,
            domains: s.domains,
            reach: s.reach,
            cors: s.cors,
            oauth: s.oauth,
        }
    }
}

impl AdminServer {
    fn new(s: Server, accounts: i64) -> Self {
        Self {
            host: host_of(&s.base_url),
            id: s.id,
            label: s.label,
            url: s.base_url,
            domains: s.domains,
            client_id: s.client_id,
            has_secret: s.has_secret,
            reach: s.reach,
            cors: s.cors,
            oauth: s.oauth,
            tested_at: s.tested_at,
            accounts,
            created_at: s.created_at,
        }
    }

    /// With the number of accounts now linked to the server.
    fn load(conn: &Connection, s: Server) -> anyhow::Result<Self> {
        let accounts = accounts::count_by_base_url(conn)?
            .get(&s.base_url)
            .copied()
            .unwrap_or(0);
        Ok(Self::new(s, accounts))
    }
}

async fn list(State(state): State<AppState>, _: CurrentUser) -> Result<Json<Value>, ApiError> {
    let (list, allow) = db::call(&state.db, |conn| {
        Ok((servers::list(conn)?, servers::allow_custom_servers(conn)?))
    })
    .await?;
    let list: Vec<PublicServer> = list.into_iter().map(Into::into).collect();
    Ok(Json(
        json!({ "servers": list, "allow_custom_servers": allow }),
    ))
}

async fn admin_list(State(state): State<AppState>, _: AdminUser) -> Result<Json<Value>, ApiError> {
    let (list, system_mail_server) = db::call(&state.db, |conn| {
        let list = servers::list(conn)?;
        let accounts = accounts::count_by_base_url(conn)?;
        // Server of the system mail (warning on removal); only if it still
        // exists. An unreadable sending account only costs the hint, not the
        // list.
        let system = system_mail::load(conn)
            .unwrap_or_else(|error| {
                tracing::warn!("system mail not readable: {error:#}");
                None
            })
            .map(|mail| mail.server_id)
            .filter(|id| list.iter().any(|s| &s.id == id));
        let list: Vec<AdminServer> = list
            .into_iter()
            .map(|s| {
                let n = accounts.get(&s.base_url).copied().unwrap_or(0);
                AdminServer::new(s, n)
            })
            .collect();
        Ok((list, system))
    })
    .await?;
    // For copying into a client created by hand.
    let redirect_uri = redirect_uri(&state.public_url);
    Ok(Json(json!({
        "servers": list,
        "redirect_uri": redirect_uri,
        "system_mail_server": system_mail_server,
    })))
}

async fn admin_get(
    State(state): State<AppState>,
    _: AdminUser,
    Path(id): Path<String>,
) -> Result<Json<AdminServer>, ApiError> {
    db::call(&state.db, move |conn| {
        servers::get(conn, &id)?
            .map(|server| AdminServer::load(conn, server))
            .transpose()
    })
    .await?
    .map(Json)
    .ok_or(ApiError::NotFound)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    label: String,
    url: String,
    #[serde(default)]
    domains: Vec<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
}

/// Partial change; missing fields stay. `client_id: null` or empty removes
/// client and secret, a new client ID without a new secret also removes the
/// secret; an empty or missing `client_secret` keeps the stored one, `null`
/// removes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    label: Option<String>,
    url: Option<String>,
    domains: Option<Vec<String>>,
    #[serde(default, deserialize_with = "nullable")]
    client_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    client_secret: Option<Option<String>>,
}

/// Validated input of a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub label: String,
    /// Origin, e.g. `https://mail.example.com`.
    pub base_url: String,
    pub domains: Vec<String>,
    pub client_id: Option<String>,
}

/// Domains: lower case, without a leading `@`, without duplicates, format
/// `name.tld`. Errors carry the first invalid domain.
pub fn normalize_domains(raw: &[String]) -> Result<Vec<String>, (&'static str, String)> {
    // Limit up front: the check runs on the database thread.
    if raw.len() > MAX_DOMAINS {
        return Err(("too_many", String::new()));
    }
    let mut domains: Vec<String> = Vec::new();
    for domain in raw {
        let domain = domain.trim().trim_start_matches('@').to_ascii_lowercase();
        if domain.is_empty() || domains.contains(&domain) {
            continue;
        }
        let labels_ok =
            domain.split('.').count() >= 2 && domain.split('.').all(crate::stalwart::is_dns_label);
        if !labels_ok || domain.len() > MAX_DOMAIN {
            return Err(("invalid", domain));
        }
        domains.push(domain);
    }
    Ok(domains)
}

pub(super) fn clean_optional(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

pub(super) fn check_client_field(errors: &mut FieldErrors, field: &str, value: Option<&str>) {
    if let Some(value) = value
        && (value.len() > MAX_CLIENT_FIELD || value.chars().any(char::is_control))
    {
        errors.code(field, "invalid");
    }
}

/// Checks fields and conflicts with the other servers (`others` without the
/// edited one): one server per origin, each domain at one server.
pub fn validate(
    label: &str,
    url: &str,
    domains: &[String],
    client_id: Option<String>,
    others: &[Server],
) -> Result<Input, FieldErrors> {
    let mut errors = FieldErrors::default();
    let label = label.trim();
    if label.is_empty() {
        errors.code("label", "required");
    } else if label.chars().count() > MAX_LABEL {
        errors.code("label", "too_long");
    }
    let base_url = match normalize_url(url) {
        Ok(url) => {
            let origin = url.origin().ascii_serialization();
            if let Some(other) = others.iter().find(|s| s.base_url == origin) {
                errors.add("url", json!({ "code": "taken", "server": other.label }));
            }
            origin
        }
        Err(code) => {
            errors.code("url", code);
            String::new()
        }
    };
    let domains = match normalize_domains(domains) {
        Ok(domains) => {
            let taken = domains.iter().find_map(|domain| {
                others
                    .iter()
                    .find(|s| s.domains.contains(domain))
                    .map(|s| (domain, s))
            });
            if let Some((domain, other)) = taken {
                errors.add(
                    "domains",
                    json!({ "code": "taken", "domain": domain, "server": other.label }),
                );
            }
            domains
        }
        Err((code, domain)) => {
            errors.add("domains", json!({ "code": code, "domain": domain }));
            Vec::new()
        }
    };
    let client_id = clean_optional(client_id);
    check_client_field(&mut errors, "client_id", client_id.as_deref());
    if errors.is_empty() {
        Ok(Input {
            label: label.to_owned(),
            base_url,
            domains,
            client_id,
        })
    } else {
        Err(errors)
    }
}

/// Entry by an administrator.
pub(super) fn audit_entry(
    category: Category,
    action: &'static str,
    user: &AuthUser,
    ip: ClientIp,
) -> Entry {
    Entry::new(category, action).actor(user).ip(ip.0)
}

fn fields(input: &Input) -> Fields<'_> {
    Fields {
        label: &input.label,
        base_url: &input.base_url,
        domains: &input.domains,
        client_id: input.client_id.as_deref(),
    }
}

async fn create(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<CreateBody>,
) -> Result<(StatusCode, Json<AdminServer>), ApiError> {
    let id = db::new_id()?;
    let secret = clean_optional(body.client_secret);
    let sealed = match &secret {
        Some(secret) => Some(state.secrets.seal(SECRET_PURPOSE, &id, secret.as_bytes())?),
        None => None,
    };
    let server = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        let others = servers::list(&tx)?;
        let input = match validate(
            &body.label,
            &body.url,
            &body.domains,
            body.client_id,
            &others,
        ) {
            Ok(input) => input,
            Err(errors) => return Ok(Err(errors)),
        };
        let mut errors = FieldErrors::default();
        check_client_field(&mut errors, "client_secret", secret.as_deref());
        if secret.is_some() && input.client_id.is_none() {
            errors.code("client_secret", "needs_client_id");
        }
        if !errors.is_empty() {
            return Ok(Err(errors));
        }
        servers::insert(&tx, &id, &fields(&input), sealed.as_deref())?;
        audit_log::insert(
            &tx,
            &audit_entry(Category::System, "stalwart.servers.create", &user, ip)
                .target(&id)
                .details(json!({ "label": input.label, "url": input.base_url })),
        )?;
        events::stalwart_servers_changed(&tx)?;
        tx.commit()?;
        let server = servers::get(conn, &id)?.ok_or_else(|| anyhow::anyhow!("server missing"))?;
        Ok(Ok(AdminServer::load(conn, server)?))
    })
    .await?
    .map_err(FieldErrors::into_error)?;
    Ok((StatusCode::CREATED, Json(server)))
}

async fn update(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<UpdateBody>,
) -> Result<Json<AdminServer>, ApiError> {
    // A new secret is encrypted up front (the ID is fixed).
    let new_secret = match &body.client_secret {
        Some(Some(secret)) if !secret.trim().is_empty() => Some(secret.trim().to_owned()),
        _ => None,
    };
    let sealed = match &new_secret {
        Some(secret) => Some(state.secrets.seal(SECRET_PURPOSE, &id, secret.as_bytes())?),
        None => None,
    };
    let result = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        let all = servers::list(&tx)?;
        let Some(current) = all.iter().find(|s| s.id == id).cloned() else {
            return Ok(None);
        };
        let others: Vec<Server> = all.into_iter().filter(|s| s.id != id).collect();
        let client_id = match body.client_id {
            None => current.client_id.clone(),
            Some(value) => value,
        };
        let input = match validate(
            body.label.as_deref().unwrap_or(&current.label),
            body.url.as_deref().unwrap_or(&current.base_url),
            body.domains.as_deref().unwrap_or(&current.domains),
            client_id,
            &others,
        ) {
            Ok(input) => input,
            Err(errors) => return Ok(Some(Err(errors))),
        };
        let mut errors = FieldErrors::default();
        check_client_field(&mut errors, "client_secret", new_secret.as_deref());
        // A secret belongs to its client: without a client ID there is none,
        // a new client ID does not keep the old one. Nor does a new address:
        // secrets never go to an address they were not entered for.
        let moved = input.base_url != current.base_url;
        let same_client = input.client_id == current.client_id && !moved;
        let secret = match (&input.client_id, sealed, &body.client_secret) {
            (None, Some(_), _) => {
                errors.code("client_secret", "needs_client_id");
                SecretChange::Clear
            }
            (None, None, _) | (Some(_), None, Some(None)) => SecretChange::Clear,
            (Some(_), Some(sealed), _) => SecretChange::Set(sealed),
            (Some(_), None, _) if same_client => SecretChange::Keep,
            (Some(_), None, _) => SecretChange::Clear,
        };
        if !errors.is_empty() {
            return Ok(Some(Err(errors)));
        }
        // The diagnosis checks address and client ID; if either changes, it
        // no longer applies.
        let reset = moved || input.client_id != current.client_id;
        servers::update(&tx, &id, &fields(&input), &secret, reset)?;
        if moved {
            if let Some(field) = system_mail::forget_secret_of(&tx, &id)? {
                // Like a change of the email settings, so the audit log shows
                // why the system mail is no longer set up.
                audit_log::insert(
                    &tx,
                    &audit_entry(Category::Settings, "system_mail.update", &user, ip)
                        .details(json!({ "fields": [field] })),
                )?;
            }
            events::stalwart_servers_changed(&tx)?;
        }
        audit_log::insert(
            &tx,
            &audit_entry(Category::System, "stalwart.servers.update", &user, ip)
                .target(&id)
                .details(json!({ "label": input.label, "url": input.base_url })),
        )?;
        tx.commit()?;
        servers::get(conn, &id)?
            .map(|server| AdminServer::load(conn, server).map(Ok))
            .transpose()
    })
    .await?;
    match result {
        None => Err(ApiError::NotFound),
        Some(Err(errors)) => Err(errors.into_error()),
        Some(Ok(server)) => Ok(Json(server)),
    }
}

/// Removes the server. Its accounts stay with their users: they count as
/// custom servers from then on (blocked while the policy is off), and the
/// system mail counts as not set up.
async fn remove(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let removed = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        let Some(server) = servers::get(&tx, &id)? else {
            return Ok(false);
        };
        servers::delete(&tx, &id)?;
        audit_log::insert(
            &tx,
            &audit_entry(Category::System, "stalwart.servers.delete", &user, ip)
                .target(&id)
                .details(json!({ "label": server.label, "url": server.base_url })),
        )?;
        events::stalwart_servers_changed(&tx)?;
        tx.commit()?;
        Ok(true)
    })
    .await?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyBody {
    allow_custom_servers: bool,
}

async fn policy(State(state): State<AppState>, _: AdminUser) -> Result<Json<PolicyBody>, ApiError> {
    let allow = db::call(&state.db, |conn| servers::allow_custom_servers(conn)).await?;
    Ok(Json(PolicyBody {
        allow_custom_servers: allow,
    }))
}

async fn set_policy(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<PolicyBody>,
) -> Result<Json<PolicyBody>, ApiError> {
    let allow = body.allow_custom_servers;
    db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        if servers::set_allow_custom_servers(&tx, allow)? {
            // Recorded under "Settings" in the audit log.
            audit_log::insert(
                &tx,
                &audit_entry(Category::Settings, "stalwart.policy", &user, ip)
                    .details(json!({ "allow_custom_servers": allow })),
            )?;
            events::stalwart_servers_changed(&tx)?;
        }
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(Json(PolicyBody {
        allow_custom_servers: allow,
    }))
}

#[derive(Serialize)]
struct TestReply {
    server: AdminServer,
    reasons: Reasons,
    /// Seconds, when the server throttled (`throttled`).
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after: Option<u64>,
}

async fn test(
    State(state): State<AppState>,
    _: AdminUser,
    Path(id): Path<String>,
) -> Result<Json<TestReply>, ApiError> {
    let lookup = id.clone();
    let server = db::call(&state.db, move |conn| servers::get(conn, &lookup))
        .await?
        .ok_or(ApiError::NotFound)?;
    let (report, recorded) = probe::test_registered(&state, &server, false).await?;
    let server = db::call(&state.db, move |conn| {
        servers::get(conn, &id)?
            .map(|server| AdminServer::load(conn, server))
            .transpose()
    })
    .await?
    .ok_or(ApiError::NotFound)?;
    // The reasons only match the state that was stored.
    let (reasons, retry_after) = if recorded {
        (report.reasons, report.retry_after.map(retry_secs))
    } else {
        (Reasons::default(), None)
    };
    Ok(Json(TestReply {
        server,
        reasons,
        retry_after,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterBody {
    username: String,
    password: String,
}

/// Registers the hub's OAuth client at the server via DCR and stores client
/// ID and secret with it (the diagnosis starts over): at most five rejected
/// sign-ins in 15 minutes (Fail2Ban of the server), errors as
/// `registration_failed` with `reason` – never 401/403, otherwise the web app
/// would consider its own session invalid. If the server changed its address
/// meanwhile, 409 `server_changed`: the client belongs to the old one.
async fn register_client(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<RegisterBody>,
) -> Result<Json<AdminServer>, ApiError> {
    let mut errors = FieldErrors::default();
    if body.username.trim().is_empty() {
        errors.code("username", "required");
    }
    if body.password.is_empty() {
        errors.code("password", "required");
    }
    if !errors.is_empty() {
        return Err(errors.into_error());
    }
    let lookup = id.clone();
    let server = db::call(&state.db, move |conn| servers::get(conn, &lookup))
        .await?
        .ok_or(ApiError::NotFound)?;
    let base_url = Url::parse(&server.base_url).map_err(anyhow::Error::from)?;
    let redirect_uri = redirect_uri(&state.public_url);
    let client_uri = state.public_url.origin().ascii_serialization();

    // If the request is aborted, the reservation returns the attempt.
    let attempt = state.limiter.reserve(&REGISTER_LIMIT, Key::Global)?;
    let result = dcr::register(
        &state.stalwart.http,
        &dcr::Registration {
            base_url: &base_url,
            redirect_uri: &redirect_uri,
            client_uri: &client_uri,
            username: body.username.trim(),
            password: &body.password,
        },
    )
    .await;
    // Only rejected sign-ins count (Fail2Ban of the server) – with a second
    // factor also those without a code, in case Stalwart counts them.
    if matches!(
        result,
        Err(dcr::Error::AuthFailed | dcr::Error::MfaRequired)
    ) {
        attempt.count();
    } else {
        drop(attempt);
    }
    let client = match result {
        Ok(client) => client,
        Err(error) => {
            tracing::info!(url = %server.base_url, reason = error.reason(), detail = ?error, "OAuth client registration failed");
            let status = match error {
                dcr::Error::AuthFailed | dcr::Error::MfaRequired | dcr::Error::Forbidden => {
                    StatusCode::UNPROCESSABLE_ENTITY
                }
                _ => StatusCode::BAD_GATEWAY,
            };
            let mut rejection =
                ApiError::rejected(status, "registration_failed").with("reason", error.reason());
            match error {
                dcr::Error::Rejected(code) => rejection = rejection.with("detail", code),
                dcr::Error::Throttled(retry_after) => {
                    rejection = rejection.with("retry_after", retry_secs(retry_after));
                }
                _ => {}
            }
            return Err(rejection);
        }
    };
    let sealed = state
        .secrets
        .seal(SECRET_PURPOSE, &id, client.client_secret.as_bytes())?;
    let client_id = client.client_id;
    let stored = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        let Some(current) = servers::get(&tx, &id)? else {
            return Ok(None);
        };
        if current.base_url != server.base_url {
            return Ok(Some(Err(())));
        }
        let fields = Fields {
            label: &current.label,
            base_url: &current.base_url,
            domains: &current.domains,
            client_id: Some(&client_id),
        };
        servers::update(&tx, &id, &fields, &SecretChange::Set(sealed), true)?;
        audit_log::insert(
            &tx,
            &audit_entry(
                Category::System,
                "stalwart.servers.register_client",
                &user,
                ip,
            )
            .target(&id)
            .details(json!({ "url": current.base_url, "client_id": client_id })),
        )?;
        tx.commit()?;
        servers::get(conn, &id)?
            .map(|server| AdminServer::load(conn, server).map(Ok))
            .transpose()
    })
    .await?;
    match stored {
        Some(Ok(server)) => Ok(Json(server)),
        // The client now exists in Stalwart without a home here; the
        // administrator can delete it there.
        Some(Err(())) => Err(ApiError::rejected(StatusCode::CONFLICT, "server_changed")),
        None => Err(ApiError::NotFound),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(label: &str, url: &str, domains: &[&str]) -> Server {
        Server {
            id: label.into(),
            label: label.into(),
            base_url: url.into(),
            domains: domains.iter().map(|d| (*d).to_owned()).collect(),
            client_id: None,
            has_secret: false,
            reach: Status::Unk,
            cors: Status::Unk,
            oauth: Status::Unk,
            tested_at: None,
            created_at: String::new(),
        }
    }

    fn code(errors: FieldErrors, field: &str) -> Value {
        errors.into_value()[field].clone()
    }

    #[test]
    fn validates_against_the_other_servers() {
        let others = [server("VPS 1", "https://mail.s1.test", &["example.com"])];
        let input = validate(
            " Mail ",
            "https://Mail.S2.test/",
            &["@Example.org".into(), "example.org".into(), " ".into()],
            Some(" roedy-hub ".into()),
            &others,
        )
        .unwrap();
        assert_eq!(
            input,
            Input {
                label: "Mail".into(),
                base_url: "https://mail.s2.test".into(),
                domains: vec!["example.org".into()],
                client_id: Some("roedy-hub".into()),
            }
        );
        let errors = validate(
            "",
            "https://mail.s1.test",
            &["example.com".into()],
            Some("a\nb".into()),
            &others,
        )
        .unwrap_err();
        let all = errors.into_value();
        assert_eq!(all["label"]["code"], "required");
        assert_eq!(all["url"], json!({ "code": "taken", "server": "VPS 1" }));
        assert_eq!(
            all["domains"],
            json!({ "code": "taken", "domain": "example.com", "server": "VPS 1" })
        );
        assert_eq!(all["client_id"]["code"], "invalid");
        let long = "x".repeat(MAX_LABEL + 1);
        let errors = validate(&long, "ftp://x", &["nodot".into()], None, &[]).unwrap_err();
        assert_eq!(code(errors, "label")["code"], "too_long");
        assert_eq!(
            normalize_domains(&vec![String::new(); MAX_DOMAINS + 1]),
            Err(("too_many", String::new()))
        );
        assert_eq!(
            normalize_domains(&["nodot".into()]),
            Err(("invalid", "nodot".into()))
        );
    }

    #[test]
    fn hosts_with_ports() {
        assert_eq!(host_of("https://mail.example.com"), "mail.example.com");
        assert_eq!(
            host_of("https://mail.example.com:8443"),
            "mail.example.com:8443"
        );
        assert_eq!(host_of("nonsense"), "");
    }
}
