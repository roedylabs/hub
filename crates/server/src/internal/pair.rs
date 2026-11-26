//! Pairing and the manifest (docs/app-contract.md › Pairing, Manifest).
//!
//! `POST /internal/v1/pair` checks, in the contract's order: the rate limit
//! (10 attempts per 15 minutes per source address, 30 in total; every
//! attempt counts), the code (unknown, used and expired codes give the same
//! `401 invalid_code`), the manifest (`400 unsupported_contract` or
//! `invalid_manifest` with `fields`, the icon sanitized like a branding
//! upload) and `public_url` (`409 public_url_mismatch` with `expected`).
//! Only a request that passes all of them uses up the code: the app can
//! correct its manifest or address and try again with the same code. The
//! source address is the TCP peer: nothing on the internal listener is a
//! trusted proxy.
//!
//! A successful pairing stores the manifest and the hash of a new secret
//! (the previous secret stops working), gives hub administrators the app's
//! admin role and, with "everyone", all other users its default role
//! (contract § 11; existing entries stay), checks the app's health at once
//! and answers with the secret and the JWKS.
//!
//! `PUT /internal/v1/manifest` stores the manifest an app sends on every
//! start. Roles it no longer has fall back to the default role (an audit
//! entry; the `access.changed` events follow with milestone 4.4); a new
//! version gets an audit entry too.

use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    Json,
    body::Bytes,
    extract::{ConnectInfo, State},
    http::StatusCode,
};
use jiff::Timestamp;
use roedy_hub_contract::{
    Manifest, ManifestError,
    manifest::SUPPORTED_CONTRACTS,
    pairing::{ManifestAccepted, Paired, normalize_pairing_code},
};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

use super::PairedApp;
use crate::{
    api::{self, ApiError},
    app::AppState,
    apps,
    client_ip::ClientIp,
    db::{
        self,
        repo::{
            app_access,
            apps::{self as repo, Status},
            audit_log::{self, Category, Entry},
        },
    },
    rate_limit::{Key, Policy},
};

/// Pairing attempts per source address.
pub const PAIR_IP: Policy = Policy::rate("internal.pair.ip", 10, Duration::from_secs(15 * 60));
/// Pairing attempts in total.
pub const PAIR_ALL: Policy = Policy::rate("internal.pair", 30, Duration::from_secs(15 * 60));

/// `401 invalid_code`.
fn invalid_code() -> ApiError {
    ApiError::rejected(StatusCode::UNAUTHORIZED, "invalid_code")
}

/// `400 invalid_manifest` with `fields`, per field `{"code": …}`.
fn invalid_manifest(fields: impl IntoIterator<Item = (String, &'static str)>) -> ApiError {
    let fields: Map<String, Value> = fields
        .into_iter()
        .map(|(field, code)| (field, json!({ "code": code })))
        .collect();
    ApiError::rejected(StatusCode::BAD_REQUEST, "invalid_manifest").with("fields", fields)
}

/// The manifest as the hub stores it: checked, with the icon sanitized.
/// Runs the SVG check off the async threads.
async fn accept_manifest(value: Option<Value>) -> Result<Manifest, ApiError> {
    let Some(value) = value else {
        return Err(invalid_manifest([("manifest".to_owned(), "required")]));
    };
    tokio::task::spawn_blocking(move || {
        let mut manifest = Manifest::parse(&value).map_err(|error| match error {
            ManifestError::UnsupportedContract => {
                ApiError::rejected(StatusCode::BAD_REQUEST, "unsupported_contract")
                    .with("supported", json!(SUPPORTED_CONTRACTS))
            }
            ManifestError::Invalid(fields) => invalid_manifest(fields),
        })?;
        manifest.icon = apps::sanitize_icon(&manifest.icon)
            .map_err(|code| invalid_manifest([("icon".to_owned(), code)]))?;
        Ok(manifest)
    })
    .await
    .map_err(|error| ApiError::Internal(error.into()))?
}

/// JSON from the body, read like axum's `Json` extractor reads the API's
/// bodies – but only once the handler wants it, so that the rate limit of
/// `pair` comes first; else `400 bad_request`. Untyped: every field is
/// checked afterwards.
fn json_value(body: &Bytes) -> Result<Value, ApiError> {
    Json::<Value>::from_bytes(body)
        .map(|Json(value)| value)
        .map_err(|rejection| ApiError::BadRequest(rejection.body_text()))
}

/// A JSON object from the body, else `400 bad_request`.
fn json_body(body: &Bytes) -> Result<Map<String, Value>, ApiError> {
    match json_value(body)? {
        Value::Object(object) => Ok(object),
        _ => Err(ApiError::BadRequest("expected a JSON object".into())),
    }
}

/// An audit entry of the category `apps` without a person: the app acted.
fn app_audit_entry(action: &'static str, slug: &str, ip: ClientIp, details: Value) -> Entry {
    Entry::new(Category::Apps, action)
        .target(slug)
        .ip(ip.0)
        .details(details)
}

/// Roles the new manifest no longer has fall back to its default role, with
/// an audit entry.
fn fall_back(
    conn: &Connection,
    app: &repo::App,
    manifest: &Manifest,
    ip: ClientIp,
) -> anyhow::Result<()> {
    let (roles, users) = app_access::fall_back_to_default(conn, &app.id, manifest)?;
    if !roles.is_empty() {
        let details = json!({
            "slug": app.slug,
            "roles": roles,
            "default": manifest.default_role().map(|role| &role.id),
            "users": users,
        });
        audit_log::insert(
            conn,
            &app_audit_entry("apps.roles_fallback", &app.slug, ip, details),
        )?;
    }
    Ok(())
}

pub async fn pair(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Result<Json<Paired>, ApiError> {
    let ip = ClientIp(peer.ip());
    state.limiter.acquire(&PAIR_IP, Key::Ip(ip.network()))?;
    state.limiter.acquire(&PAIR_ALL, Key::Global)?;
    let mut request = json_body(&body)?;

    let code = request
        .get("code")
        .and_then(Value::as_str)
        .and_then(normalize_pairing_code)
        .ok_or_else(invalid_code)?;
    let code_hash = apps::hash(&code);
    let app = db::call(&state.db, move |conn| {
        Ok(match repo::app_for_code(conn, &code_hash)? {
            Some(id) => repo::by_id(conn, &id)?,
            None => None,
        })
    })
    .await?
    .ok_or_else(invalid_code)?;

    let manifest = accept_manifest(request.remove("manifest")).await?;

    let expected = api::apps::public_url(&state, &app.slug);
    if request.get("public_url").and_then(Value::as_str) != Some(expected.as_str()) {
        return Err(
            ApiError::rejected(StatusCode::CONFLICT, "public_url_mismatch")
                .with("expected", expected),
        );
    }

    let secret = apps::new_secret(&app.id)?;
    let secret_hash = apps::hash(&secret);
    let table = state.apps.clone();
    let paired = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        // The code may have been used or replaced since the check.
        let Some(app_id) = repo::take_code(&tx, &code_hash)? else {
            return Ok(Err(invalid_code()));
        };
        let Some(app) = repo::by_id(&tx, &app_id)? else {
            return Ok(Err(invalid_code()));
        };
        let again = app.status == Status::Paired;
        repo::pair(&tx, &app.id, &manifest, &secret_hash)?;
        if again {
            fall_back(&tx, &app, &manifest, ip)?;
        }
        let granted = app_access::grant_after_pairing(&tx, &app.id, &manifest)?;
        let details = json!({
            "slug": app.slug,
            "version": manifest.version,
            "manifest_id": manifest.id,
            "name": manifest.name_in("en"),
            "again": again,
            "granted": granted,
        });
        audit_log::insert(&tx, &app_audit_entry("apps.pair", &app.slug, ip, details))?;
        tx.commit()?;
        api::apps::reload(&table, conn);
        Ok(Ok(app))
    })
    .await??;
    tracing::info!(slug = paired.slug, public_url = expected, "app paired");

    // "Connected" in the administration as soon as the app answers.
    let checker = state.clone();
    let slug = paired.slug.clone();
    state.background.spawn(async move {
        if let Err(error) = crate::jobs::health_poll::check_one(&checker, &slug).await {
            tracing::warn!(slug, "health check after pairing failed: {error:#}");
        }
    });

    Ok(Json(Paired {
        app_id: paired.id,
        public_url: api::apps::public_url(&state, &paired.slug),
        slug: paired.slug,
        app_secret: secret,
        issuer: state.identities.issuer().to_owned(),
        jwks: state.keys.jwks(Timestamp::now()),
        contract: 1,
    }))
}

pub async fn put_manifest(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    PairedApp(app, secret_hash): PairedApp,
    body: Bytes,
) -> Result<Json<ManifestAccepted>, ApiError> {
    let ip = ClientIp(peer.ip());
    let manifest = accept_manifest(Some(json_value(&body)?)).await?;
    let table = state.apps.clone();
    let app = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        // Removed or paired anew since the secret was checked: the old
        // secret must not overwrite what the new pairing stored.
        let Some(current) = repo::by_secret_hash(&tx, &secret_hash)?.filter(|c| c.id == app.id)
        else {
            return Ok(Err(super::invalid_credentials()));
        };
        let previous = current.manifest.as_ref().map(|m| m.version.clone());
        repo::set_manifest(&tx, &current.id, &manifest)?;
        if previous.as_deref() != Some(manifest.version.as_str()) {
            let details = json!({
                "slug": current.slug,
                "version": manifest.version,
                "previous": previous,
            });
            audit_log::insert(
                &tx,
                &app_audit_entry("apps.update", &current.slug, ip, details),
            )?;
        }
        fall_back(&tx, &current, &manifest, ip)?;
        tx.commit()?;
        api::apps::reload(&table, conn);
        Ok(Ok(current))
    })
    .await??;
    Ok(Json(ManifestAccepted {
        app_id: app.id,
        public_url: api::apps::public_url(&state, &app.slug),
        slug: app.slug,
        issuer: state.identities.issuer().to_owned(),
    }))
}
