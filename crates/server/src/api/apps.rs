//! Administration › Apps, as far as pairing needs it (docs/app-contract.md
//! › Pairing); milestone 5 adds order, access default and access per user.
//!
//! - `GET admin/apps`: all apps in their order with state and health.
//! - `POST admin/apps` (`{slug, internal_url}`): adds a pending app and
//!   returns it with its first pairing code (`201`). The slug follows the
//!   contract's format and is not reserved (422 `invalid` with
//!   `fields.slug`: `required`, `invalid`, `reserved`, `taken`); the internal
//!   address is scheme, host and port (`fields.internal_url`).
//! - `GET admin/apps/:slug`: one app.
//! - `POST admin/apps/:slug/pairing-code`: a new code for the app (pending or
//!   paired: "New pairing code"); it replaces the open one. The code is shown
//!   only in this response, valid for 15 minutes.
//! - `DELETE admin/apps/:slug`: removes the app; its secret, pairing code,
//!   access entries and events go with it, and its event delivery stops.
//!
//! Every change runs in one transaction with `invites::admin_holds` and an
//! audit entry in the category `apps`, and reloads the app table.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use roedy_hub_contract::pairing::display_pairing_code;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    ApiError, FieldErrors, JsonBody,
    invites::{Actor, admin_holds},
};
use crate::{
    app::AppState,
    apps::{self, AppTable},
    auth::{AdminUser, AuthUser},
    client_ip::ClientIp,
    db::{
        self,
        repo::{
            apps::{self as repo, App},
            audit_log::{self, Category, Entry},
        },
    },
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/apps", get(list).post(create))
        .route("/admin/apps/{slug}", get(show).delete(remove))
        .route("/admin/apps/{slug}/pairing-code", post(pairing_code))
}

/// The app's public address: `<hub public URL>/<slug>`.
pub fn public_url(state: &AppState, slug: &str) -> String {
    format!("{}/{slug}", state.identities.issuer())
}

/// An app for the administration.
pub fn app_json(state: &AppState, app: &App) -> Value {
    let manifest = app.manifest.as_ref();
    json!({
        "id": app.id,
        "slug": app.slug,
        "internal_url": app.internal_url,
        "public_url": public_url(state, &app.slug),
        "state": app.status.as_str(),
        "manifest_id": manifest.map(|m| &m.id),
        "version": manifest.map(|m| &m.version),
        "name": manifest.map(|m| &m.name),
        "description": manifest.map(|m| &m.description),
        "home": manifest.map(|m| &m.home),
        "roles": manifest.map(|m| m.roles.iter().map(|role| json!({
            "id": role.id,
            "name": role.name,
            "description": role.description,
            "default": role.default,
            "admin": role.admin,
        })).collect::<Vec<_>>()),
        "access_default": app.access_default.as_str(),
        "position": app.position,
        "health": app.health.map(repo::Health::as_str),
        "health_at": app.health_at,
        "paired_at": app.paired_at,
        "created_at": app.created_at,
        "pairing": app.pairing_expires_at.as_ref().map(|at| json!({ "expires_at": at })),
    })
}

/// An audit entry of the category `apps`.
fn app_entry(actor: &Actor<'_>, action: &'static str, slug: &str, details: Value) -> Entry {
    Entry::new(Category::Apps, action)
        .actor(actor.admin)
        .ip(actor.ip)
        .target(slug)
        .details(details)
}

async fn list(State(state): State<AppState>, _: AdminUser) -> Result<Json<Value>, ApiError> {
    let apps = db::call(&state.db, |conn| Ok(repo::list(conn)?)).await?;
    let rows: Vec<Value> = apps.iter().map(|app| app_json(&state, app)).collect();
    Ok(Json(json!({ "apps": rows })))
}

async fn show(
    State(state): State<AppState>,
    _: AdminUser,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let app = db::call(&state.db, move |conn| Ok(repo::by_slug(conn, &slug)?))
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({ "app": app_json(&state, &app) })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewApp {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    internal_url: String,
}

/// The code for the response: canonical and in groups of four.
fn pairing_json(code: &str, expires_at: &str) -> Value {
    json!({
        "code": code,
        "display": display_pairing_code(code),
        "expires_at": expires_at,
    })
}

/// A new code for the app, stored as its only open one; returns the
/// expiry.
fn store_code(
    conn: &Connection,
    actor: &Actor<'_>,
    app_id: &str,
    code: &str,
) -> rusqlite::Result<String> {
    repo::set_pairing_code(conn, app_id, &apps::hash(code), Some(&actor.admin.id))
}

/// Reloads the app table after a commit: a failed reload leaves the table
/// at its previous state, which the next change corrects (logged).
pub(crate) fn reload(table: &AppTable, conn: &Connection) {
    if let Err(error) = table.reload(conn) {
        tracing::error!("app table not reloaded: {error:#}");
    }
}

async fn create(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<NewApp>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let slug = body.slug.trim().to_owned();
    let mut errors = FieldErrors::default();
    if slug.is_empty() {
        errors.code("slug", "required");
    } else if let Err(code) = apps::check_slug(&slug) {
        errors.code("slug", code);
    }
    let internal_url = if body.internal_url.trim().is_empty() {
        errors.code("internal_url", "required");
        None
    } else {
        let origin = apps::internal_origin(&body.internal_url);
        if origin.is_none() {
            errors.code("internal_url", "invalid");
        }
        origin
    };
    if !errors.is_empty() {
        return Err(errors.into_error());
    }
    let internal_url = internal_url.unwrap_or_default();
    let code = apps::new_pairing_code()?;
    let (app, pairing) = change(&state, admin, ip, move |tx, actor| {
        if repo::slug_taken(tx, &slug)? {
            let mut errors = FieldErrors::default();
            errors.code("slug", "taken");
            return Ok(Err(errors.into_error()));
        }
        let id = repo::insert(tx, &slug, &internal_url)?;
        let expires_at = store_code(tx, actor, &id, &code)?;
        let details = json!({ "slug": slug, "internal_url": internal_url });
        audit_log::insert(tx, &app_entry(actor, "apps.create", &slug, details))?;
        let app = repo::by_id(tx, &id)?.ok_or_else(|| anyhow::anyhow!("app gone"))?;
        Ok(Ok((app, expires_at, code)))
    })
    .await
    .map(|(app, expires_at, code)| (app, pairing_json(&code, &expires_at)))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "app": app_json(&state, &app), "pairing": pairing })),
    ))
}

async fn pairing_code(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let code = apps::new_pairing_code()?;
    let (app, expires_at, code) = change(&state, admin, ip, move |tx, actor| {
        let Some(app) = repo::by_slug(tx, &slug)? else {
            return Ok(Err(ApiError::NotFound));
        };
        let expires_at = store_code(tx, actor, &app.id, &code)?;
        let details = json!({ "slug": slug });
        audit_log::insert(tx, &app_entry(actor, "apps.pairing_code", &slug, details))?;
        let app = repo::by_id(tx, &app.id)?.ok_or_else(|| anyhow::anyhow!("app gone"))?;
        Ok(Ok((app, expires_at, code)))
    })
    .await?;
    Ok(Json(json!({
        "app": app_json(&state, &app),
        "pairing": pairing_json(&code, &expires_at),
    })))
}

async fn remove(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(slug): Path<String>,
) -> Result<StatusCode, ApiError> {
    change(&state, admin, ip, move |tx, actor| {
        let Some(app) = repo::by_slug(tx, &slug)? else {
            return Ok(Err(ApiError::NotFound));
        };
        repo::delete(tx, &app.id)?;
        let details = json!({
            "slug": slug,
            "name": app.manifest.as_ref().map(|m| m.name_in("en")),
        });
        audit_log::insert(tx, &app_entry(actor, "apps.remove", &slug, details))?;
        Ok(Ok(()))
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A change by an administrator in one transaction, like
/// `invites::admin_change`: first `admin_holds`, then `apply`; after the
/// commit the app table is reloaded in the same database job, so that a
/// cancelled request cannot leave it behind.
async fn change<T: Send + 'static>(
    state: &AppState,
    admin: AuthUser,
    ip: ClientIp,
    apply: impl FnOnce(&Connection, &Actor<'_>) -> anyhow::Result<Result<T, ApiError>> + Send + 'static,
) -> Result<T, ApiError> {
    let table = state.apps.clone();
    db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        if let Err(error) = admin_holds(&tx, &admin)? {
            return Ok(Err(error));
        }
        let actor = Actor {
            admin: &admin,
            ip: ip.0,
        };
        let result = apply(&tx, &actor)?;
        if result.is_ok() {
            tx.commit()?;
            reload(&table, conn);
        }
        Ok(result)
    })
    .await?
}
