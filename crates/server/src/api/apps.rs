//! Administration › Apps (docs/app-contract.md › Pairing, Access and
//! roles).
//!
//! - `GET admin/apps`: all apps in their order with their status: pairing
//!   state, version, health (`health`, `health_at`, `online_at`: the last
//!   passed check), `contacted_at` (the app's last request with its secret,
//!   to the minute), the event delivery (`events`), the number of users
//!   with access (`user_count`), the access default, the manifest's roles,
//!   the address of its home (`url`) and its icon (`icon`, versioned).
//! - `PATCH admin/apps` (`{order: [slug, …]}`): the order on the home page;
//!   `order` names every app exactly once (422 `fields.order`: `required`,
//!   `invalid`). Returns the list.
//! - `POST admin/apps` (`{slug, internal_url}`): adds a pending app and
//!   returns it with its first pairing code (`201`). The slug follows the
//!   contract's format and is not reserved (422 `invalid` with
//!   `fields.slug`: `required`, `invalid`, `reserved`, `taken`); the internal
//!   address is scheme, host and port (`fields.internal_url`).
//! - `GET admin/apps/:slug`: one app.
//! - `PATCH admin/apps/:slug` (`{access_default?, enabled?}`, at least one;
//!   422 `fields.access_default: required` without either):
//!   - `access_default`: `all` ("everyone": new users get the default role)
//!     or `selected` ("selected only": new users get none). It applies to
//!     users created from then on – accepted invitations without their own
//!     choice for the app included; existing access stays exactly as it
//!     is, in both directions (contract § 11). The administration grants or
//!     removes it per user (`users`).
//!   - `enabled`: switches the app off (`false`) or on again (`true`;
//!     docs/plan.md § 9.1). Off, the gateway answers `503 app_disabled`,
//!     `GET apps` leaves it out, its open streams and WebSockets close, its
//!     events wait in the outbox and its health is not checked; pairing,
//!     access, settings and its data stay, and its internal API keeps
//!     working except for Stalwart access tokens (contract § 7.1). On
//!     again, a failing delivery's backoff is reset, the waiting events go
//!     out at once and its health is checked right away. Audit
//!     `apps.disable` / `apps.enable`.
//! - `GET admin/apps/:slug/users`: every user (active and locked) with
//!   their role in the app (`app_role`, `null` without access) and every
//!   open invitation with the role it will give.
//! - `POST admin/apps/:slug/pairing-code`: a new code for the app (pending or
//!   paired: "New pairing code"); it replaces the open one. The code is shown
//!   only in this response, valid for 15 minutes.
//! - `DELETE admin/apps/:slug`: removes the app; its secret, pairing code,
//!   access entries, the invitations' choices and events go with it, and its
//!   event delivery stops.
//!
//! Every change runs in one transaction with `invites::admin_holds` and an
//! audit entry in the category `apps`, and reloads the app table. The
//! access of one user ([`set_access`]) is changed through `users`.

use std::collections::{BTreeSet, HashMap};

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
            app_access::{self, Outcome},
            apps::{self as repo, AccessDefault, App, Status},
            audit_log::{self, Category, Entry},
            events, invitations,
            users::Profile,
        },
    },
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/apps", get(list).post(create).patch(reorder))
        .route("/admin/apps/{slug}", get(show).patch(update).delete(remove))
        .route("/admin/apps/{slug}/users", get(users))
        .route("/admin/apps/{slug}/pairing-code", post(pairing_code))
}

/// The app's public address: `<hub public URL>/<slug>`.
pub fn public_url(state: &AppState, slug: &str) -> String {
    format!("{}/{slug}", state.identities.issuer())
}

/// What the administration shows besides the stored app.
#[derive(Debug, Default)]
pub struct Extra {
    /// Users with access.
    pub users: u64,
    /// State of the event delivery; `None` before the first event.
    pub events: Option<events::State>,
}

/// [`Extra`] for every app, by ID.
pub fn extras(conn: &Connection) -> rusqlite::Result<HashMap<String, Extra>> {
    let mut extras: HashMap<String, Extra> = HashMap::new();
    for (app, users) in app_access::counts(conn)? {
        extras.entry(app).or_default().users = users;
    }
    for (app, state) in events::states(conn)? {
        extras.entry(app).or_default().events = Some(state);
    }
    Ok(extras)
}

/// The app with its [`Extra`], for one app.
fn with_extra(conn: &Connection, id: &str) -> anyhow::Result<(App, Extra)> {
    let app = repo::by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("app gone"))?;
    let extra = extras(conn)?.remove(id).unwrap_or_default();
    Ok((app, extra))
}

/// The event delivery: `pending` events not yet acknowledged; while
/// deliveries fail `failing_since`, `attempts`, `last_error` and
/// `retry_at` (`null` with `given_up`: past 7 days the hub waits until the
/// app is back).
fn events_json(state: Option<&events::State>) -> Value {
    let Some(state) = state else {
        return json!({
            "pending": 0, "attempts": 0, "failing_since": null, "retry_at": null,
            "last_error": null, "given_up": false,
        });
    };
    json!({
        "pending": state.last_seq.saturating_sub(state.delivered_seq),
        "attempts": state.attempts,
        "failing_since": state.failing_since,
        "retry_at": state.retry_at,
        "last_error": state.last_error,
        "given_up": state.attempts > 0 && state.retry_at.is_none(),
    })
}

/// An app for the administration.
pub fn app_json(state: &AppState, app: &App, extra: &Extra) -> Value {
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
        "url": manifest.map(|m| format!("/{}{}", app.slug, m.home)),
        "icon": manifest.map(|m| apps::icon_url(&app.slug, m)),
        "roles": manifest.map(|m| m.roles.iter().map(|role| json!({
            "id": role.id,
            "name": role.name,
            "description": role.description,
            "default": role.default,
            "admin": role.admin,
        })).collect::<Vec<_>>()),
        "access_default": app.access_default.as_str(),
        "enabled": app.enabled(),
        "disabled_at": app.disabled_at,
        "position": app.position,
        "health": app.health.map(repo::Health::as_str),
        "health_at": app.health_at,
        "online_at": app.online_at,
        "contacted_at": app.contacted_at,
        "paired_at": app.paired_at,
        "created_at": app.created_at,
        "pairing": app.pairing_expires_at.as_ref().map(|at| json!({ "expires_at": at })),
        "user_count": extra.users,
        "events": events_json(extra.events.as_ref()),
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

/// Every app with its [`Extra`], in their order.
fn all(conn: &Connection) -> anyhow::Result<(Vec<App>, HashMap<String, Extra>)> {
    Ok((repo::list(conn)?, extras(conn)?))
}

fn list_json(state: &AppState, apps: &[App], extras: &HashMap<String, Extra>) -> Value {
    let none = Extra::default();
    let rows: Vec<Value> = apps
        .iter()
        .map(|app| app_json(state, app, extras.get(&app.id).unwrap_or(&none)))
        .collect();
    json!({ "apps": rows })
}

async fn list(State(state): State<AppState>, _: AdminUser) -> Result<Json<Value>, ApiError> {
    let (apps, extras) = db::call(&state.db, |conn| all(conn)).await?;
    Ok(Json(list_json(&state, &apps, &extras)))
}

async fn show(
    State(state): State<AppState>,
    _: AdminUser,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (app, extra) = db::call(&state.db, move |conn| {
        let Some(app) = repo::by_slug(conn, &slug)? else {
            return Ok(None);
        };
        Ok(Some(with_extra(conn, &app.id)?))
    })
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({ "app": app_json(&state, &app, &extra) })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderBody {
    order: Vec<String>,
}

/// "Order" of the list (drag and drop or "Move up/down"): every slug once,
/// in the new order. Checked in the transaction against the apps that
/// exist then: one added or removed meanwhile makes it `invalid`, and the
/// page loads the list again.
async fn reorder(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<OrderBody>,
) -> Result<Json<Value>, ApiError> {
    if body.order.is_empty() {
        let mut errors = FieldErrors::default();
        errors.code("order", "required");
        return Err(errors.into_error());
    }
    let (apps, extras) = change(&state, admin, ip, move |tx, actor| {
        let apps = repo::list(tx)?;
        let by_slug: HashMap<&str, &App> =
            apps.iter().map(|app| (app.slug.as_str(), app)).collect();
        let unique: BTreeSet<&str> = body.order.iter().map(String::as_str).collect();
        if unique.len() != body.order.len()
            || body.order.len() != apps.len()
            || !unique.iter().all(|slug| by_slug.contains_key(slug))
        {
            let mut errors = FieldErrors::default();
            errors.code("order", "invalid");
            return Ok(Err(errors.into_error()));
        }
        let ids: Vec<String> = body
            .order
            .iter()
            .map(|slug| by_slug[slug.as_str()].id.clone())
            .collect();
        if repo::set_order(tx, &ids)? {
            let previous: Vec<&str> = apps.iter().map(|app| app.slug.as_str()).collect();
            let entry = Entry::new(Category::Apps, "apps.order")
                .actor(actor.admin)
                .ip(actor.ip)
                .details(json!({ "order": body.order, "previous": previous }));
            audit_log::insert(tx, &entry)?;
        }
        Ok(Ok(all(tx)?))
    })
    .await?;
    Ok(Json(list_json(&state, &apps, &extras)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    #[serde(default)]
    access_default: Option<AccessDefault>,
    #[serde(default)]
    enabled: Option<bool>,
}

/// "Access for new users" ("Everyone" or "Selected users only"; existing
/// access stays as it is) and "Switched on" (module documentation).
async fn update(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(slug): Path<String>,
    JsonBody(body): JsonBody<UpdateBody>,
) -> Result<Json<Value>, ApiError> {
    if body.access_default.is_none() && body.enabled.is_none() {
        let mut errors = FieldErrors::default();
        errors.code("access_default", "required");
        return Err(errors.into_error());
    }
    let (app, extra, switched) = change(&state, admin, ip, move |tx, actor| {
        let Some(app) = repo::by_slug(tx, &slug)? else {
            return Ok(Err(ApiError::NotFound));
        };
        if let Some(access) = body.access_default
            && repo::set_access_default(tx, &app.id, access)?
        {
            let details = json!({
                "slug": slug,
                "access_default": access.as_str(),
                "previous": app.access_default.as_str(),
            });
            audit_log::insert(tx, &app_entry(actor, "apps.access_default", &slug, details))?;
        }
        let mut switched = None;
        if let Some(enabled) = body.enabled
            && repo::set_enabled(tx, &app.id, enabled)?
        {
            if enabled {
                // The waiting events go out at once, not after a backoff
                // that ran out while the app was off.
                events::resume(tx, &app.id, false)?;
            }
            let action = if enabled {
                "apps.enable"
            } else {
                "apps.disable"
            };
            let details = json!({
                "slug": slug,
                "name": app.manifest.as_ref().map(|m| m.name_in("en")),
            });
            audit_log::insert(tx, &app_entry(actor, action, &slug, details))?;
            switched = Some(enabled);
        }
        let (app, extra) = with_extra(tx, &app.id)?;
        Ok(Ok((app, extra, switched)))
    })
    .await?;
    match switched {
        Some(false) => {
            tracing::info!(slug = app.slug, "app switched off");
            state.gateway.streams.end_app(&app.id);
        }
        Some(true) => {
            tracing::info!(slug = app.slug, "app switched on");
            state.events.wake();
            let checker = state.clone();
            let slug = app.slug.clone();
            state.background.spawn(async move {
                if let Err(error) = crate::jobs::health_poll::check_one(&checker, &slug).await {
                    tracing::warn!(slug, "health check after switching on failed: {error:#}");
                }
            });
        }
        None => {}
    }
    Ok(Json(json!({ "app": app_json(&state, &app, &extra) })))
}

/// "Access" of the app's page: the users with their role in the app, and
/// what the open invitations will give.
async fn users(
    State(state): State<AppState>,
    _: AdminUser,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let found = db::call(&state.db, move |conn| {
        let Some(app) = repo::by_slug(conn, &slug)? else {
            return Ok(None);
        };
        let users = app_access::users_of(conn, &app.id)?;
        let invites = invitations::list(conn)?;
        let choices = app_access::all_choices(conn)?;
        Ok(Some((app, users, invites, choices)))
    })
    .await?;
    let (app, users, invites, choices) = found.ok_or(ApiError::NotFound)?;
    let users: Vec<Value> = users
        .iter()
        .map(|user| {
            let p = &user.profile;
            json!({
                "id": p.id,
                "status": if user.active { "active" } else { "locked" },
                "name": p.display_name,
                "email": p.email,
                "username": p.username,
                "role": p.role.as_str(),
                "app_role": user.app_role,
            })
        })
        .collect();
    let invites: Vec<Value> = invites
        .iter()
        .map(|invite| {
            let choice = choices.get(&invite.id).and_then(|c| c.get(&app.id));
            json!({
                "id": invite.id,
                "status": "invited",
                "name": invite.name,
                "email": invite.email,
                "role": invite.role.as_str(),
                "app_role": app_access::effective_for(&app, choice),
            })
        })
        .collect();
    Ok(Json(json!({ "users": users, "invites": invites })))
}

/// 409 `app_pending`: access is only given in paired apps (a pending app
/// has no roles yet; hub administrators reach it anyway).
pub(super) fn app_pending() -> ApiError {
    ApiError::rejected(StatusCode::CONFLICT, "app_pending")
}

/// The role for access to a paired app: `requested` if the manifest has it
/// (otherwise `Err` – the caller names the field), without one the default
/// role.
pub(super) fn role_in(app: &App, requested: Option<&str>) -> Result<String, ()> {
    let manifest = app.manifest.as_ref().ok_or(())?;
    match requested {
        Some(role) if manifest.has_role(role) => Ok(role.to_owned()),
        Some(_) => Err(()),
        None => manifest
            .default_role()
            .map(|role| role.id.clone())
            .ok_or(()),
    }
}

/// Changes one user's access to one paired app (`role`: with that role,
/// `None`: none), with `access.changed` for the app and an audit entry –
/// `apps.access_grant`, `apps.access_role` or `apps.access_revoke`. `false`
/// if it already applied. After the commit the caller lets the gateway
/// read the user's roles again ([`access_changed`]).
pub(super) fn set_access(
    conn: &Connection,
    actor: &Actor<'_>,
    user: &Profile,
    app: &App,
    role: Option<&str>,
) -> anyhow::Result<bool> {
    debug_assert_eq!(app.status, Status::Paired);
    let Outcome::Changed { previous } = app_access::set(conn, &user.id, &app.id, role)? else {
        return Ok(false);
    };
    let action = match (&previous, role) {
        (None, _) => "apps.access_grant",
        (Some(_), Some(_)) => "apps.access_role",
        (Some(_), None) => "apps.access_revoke",
    };
    let entry = Entry::new(Category::Apps, action)
        .actor(actor.admin)
        .ip(actor.ip)
        .target(&user.id)
        .details(json!({
            "slug": app.slug,
            "name": user.display_name,
            "email": user.email,
            "app_role": role,
            "previous": previous,
        }));
    audit_log::insert(conn, &entry)?;
    Ok(true)
}

/// After the commit of an access change: the gateway reads the user's roles
/// again with the next request, and without access the user's open streams
/// and WebSockets to the app close.
pub(super) fn access_changed(state: &AppState, user_id: &str, app_id: &str, access: bool) {
    state.gateway.sessions.forget_user(user_id, None);
    if !access {
        state.gateway.streams.end_user_app(user_id, app_id);
    }
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
        let app = with_extra(tx, &id)?;
        Ok(Ok((app, expires_at, code)))
    })
    .await
    .map(|(app, expires_at, code)| (app, pairing_json(&code, &expires_at)))?;
    let (app, extra) = app;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "app": app_json(&state, &app, &extra), "pairing": pairing })),
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
        let app = with_extra(tx, &app.id)?;
        Ok(Ok((app, expires_at, code)))
    })
    .await?;
    let (app, extra) = app;
    Ok(Json(json!({
        "app": app_json(&state, &app, &extra),
        "pairing": pairing_json(&code, &expires_at),
    })))
}

async fn remove(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(slug): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = change(&state, admin, ip, move |tx, actor| {
        let Some(app) = repo::by_slug(tx, &slug)? else {
            return Ok(Err(ApiError::NotFound));
        };
        repo::delete(tx, &app.id)?;
        let details = json!({
            "slug": slug,
            "name": app.manifest.as_ref().map(|m| m.name_in("en")),
        });
        audit_log::insert(tx, &app_entry(actor, "apps.remove", &slug, details))?;
        Ok(Ok(app.id))
    })
    .await?;
    // Forwarding stopped with the reload; open streams go as well.
    state.gateway.streams.end_app(&id);
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
