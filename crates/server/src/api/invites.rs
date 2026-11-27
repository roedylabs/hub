//! Invitations (Administration › Users and the invitation page).
//!
//! For administrators (`invites/:id`):
//!
//! - `GET invites`: the open invitations, oldest first.
//! - `POST invites` (`{emails, role, name?, apps?}`): creates every invitation, even
//!   if its mail does not go out, and reports ID and `sent` per address;
//!   without a system mail the invitations are only created
//!   (`system_mail: false`, "Copy link" then passes them on). An address is
//!   taken if it belongs to a user (any status) or an open invitation
//!   (`emails.<i>`: `taken`). An open email address change to it does not
//!   count; the invitation ends its link for good.
//! - `POST invites/:id/resend`: the same link; on success a new expiry
//!   (7 days) and `mail_sent_at`, otherwise 502 `mail_failed` without change.
//! - `POST invites/:id/renew` ("Extend"): the same link, new expiry
//!   (7 days), no mail; `mail_sent_at` stays. As with `resend`, the acting
//!   administrator becomes the inviter. Also for valid invitations (the page
//!   only offers it for expired ones).
//! - `GET invites/:id/link` ("Copy link"), `PATCH invites/:id`
//!   (`{role?, apps?}`), `DELETE invites/:id` (revoke).
//!
//! App access in the form: `apps` maps the slug of a paired app to a role
//! from its manifest or to `null` for "no access"; apps it does not name get
//! their default when the invitation is accepted (contract § 11: "everyone"
//! the default role, "selected only" none). `PATCH` changes only the apps it
//! names. Errors per app under `apps.<slug>`: `not_found`, `app_pending`,
//! `invalid` (no such role). Every invitation in a response carries `apps`:
//! slug → role of the access accepting it will give, as of now (a role the
//! manifest dropped meanwhile becomes its default role). Choices are
//! recorded in the audit log (`apps.invite_access`, category `apps`);
//! accepting grants the access with `access.changed` for each app.
//!
//! Public with the token from the link (`invites/:token`; unknown tokens are
//! 404 and count towards the per-IP limit like the other links from mails):
//!
//! - `GET`: address, name, role, inviter, expiry (`expired`).
//! - `POST …/totp`: secret for the optional "Multi-factor" step.
//! - `POST …/accept` (`{name, password, code?}`): with `code` (from the app
//!   of the "Multi-factor" step) the account gets that app and recovery
//!   codes, without it only the password. In one transaction the account,
//!   the first app, the codes and the session; the invitation is gone
//!   afterwards. 201 with the codes (`null` without an app) and the session
//!   cookie.
//! - `POST …/request`: "Request a new invitation" for an expired invitation
//!   (otherwise 422 `invite_valid`); always 202. At most once per 24 hours
//!   it is recorded (`requested_at` in the list, `users.invite_request` in
//!   the audit log).
//!
//! Taken over from Roedy Mail (`crates/server/src/api/invites.rs`) without
//! the notifications in the app (a request shows in the list instead); the
//! multi-factor step is optional here.

use std::{
    collections::{BTreeMap, HashSet},
    net::IpAddr,
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse as _, Response},
    routing::{get, post},
};
use jiff::Timestamp;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    ApiError, FieldErrors, JsonBody, auth,
    me::{check_display_name, check_email, check_new_password},
    password::LINK_IP,
};
use crate::{
    app::AppState,
    auth::{AdminUser, AuthUser, CurrentUser, recovery, session, session::Token, totp},
    client_ip::ClientIp,
    db::{
        self,
        repo::{
            app_access::{self, Outcome},
            apps::{self as app_repo, App as HubApp, Status},
            audit_log::{self, Category, Entry},
            email_changes,
            events::Reason,
            invitations::{self, Found, Invitation, NewInvitation, TOKEN_PURPOSE, TOTP_PURPOSE},
            known_devices, sessions, totp_devices,
            users::{self, NewAccess, NewUser, Role},
        },
    },
    mailer::{
        delivery::{Delivery, Unavailable},
        format::{Format, full_name},
        mail_failed,
        templates::Invite,
    },
    rate_limit::{Key, Policy},
    settings,
};

/// At most this many addresses at once.
const MAX_INVITES: usize = 50;
/// Invitation mails per administrator: 50 at once, then one per 72 seconds
/// (also "Resend") – a hijacked session should not be able to write to any
/// number of addresses through the system mail.
const INVITE_MAIL: Policy = Policy::rate("invites.mail", 50, Duration::from_secs(60 * 60));
/// Wrong codes in the "Multi-factor" step per invitation: five in
/// 15 minutes, then a 15-minute lockout (like `auth.login.code`).
const INVITE_CODE: Policy = Policy::lockout(
    "invites.code",
    5,
    Duration::from_secs(15 * 60),
    Duration::from_secs(15 * 60),
);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/invites", get(list).post(create))
        .route("/invites/{key}", get(show).patch(update).delete(revoke))
        .route("/invites/{key}/resend", post(resend))
        .route("/invites/{key}/renew", post(renew))
        .route("/invites/{key}/link", get(link))
        .route("/invites/{key}/totp", post(enroll))
        .route("/invites/{key}/accept", post(accept))
        .route("/invites/{key}/request", post(request))
}

fn rejected(code: &'static str) -> ApiError {
    ApiError::rejected(StatusCode::UNPROCESSABLE_ENTITY, code)
}

/// 422 `invite_expired`: the page shows "Invitation expired".
fn expired() -> ApiError {
    rejected("invite_expired")
}

/// Who acts: for the audit log.
pub(super) struct Actor<'a> {
    pub admin: &'a AuthUser,
    pub ip: IpAddr,
}

impl Actor<'_> {
    pub fn entry(&self, action: &'static str, target: &str, details: Value) -> Entry {
        Entry::new(Category::Users, action)
            .actor(self.admin)
            .ip(self.ip)
            .target(target)
            .details(details)
    }
}

/// New role of an invitation with audit entry; `false` if it already applied.
pub(super) fn change_role(
    conn: &Connection,
    actor: &Actor<'_>,
    invite: &Invitation,
    role: Role,
) -> anyhow::Result<bool> {
    if invite.role == role || !invitations::set_role(conn, &invite.id, role)? {
        return Ok(false);
    }
    let details = json!({ "email": invite.email, "role": role.as_str() });
    audit_log::insert(conn, &actor.entry("users.invite_role", &invite.id, details))?;
    Ok(true)
}

/// Revokes the invitation: the link no longer works.
pub(super) fn revoke_invite(
    conn: &Connection,
    actor: &Actor<'_>,
    invite: &Invitation,
) -> anyhow::Result<bool> {
    if !invitations::delete(conn, &invite.id)? {
        return Ok(false);
    }
    let details = json!({ "email": invite.email });
    audit_log::insert(
        conn,
        &actor.entry("users.invite_revoke", &invite.id, details),
    )?;
    Ok(true)
}

/// Whether the acting administrator may still act: their session exists, the
/// account is active and has the administrator role (otherwise 401 or 403
/// like before the request). Taken over from Roedy Mail (`api::users`).
pub(super) fn admin_holds(
    conn: &Connection,
    admin: &AuthUser,
) -> rusqlite::Result<Result<(), ApiError>> {
    if !sessions::is_valid(conn, &admin.id, &admin.session_id)? {
        return Ok(Err(ApiError::Unauthorized));
    }
    Ok(match users::credentials_by_id(conn, &admin.id)? {
        Some(now) if now.active && now.user.role == Role::Admin => Ok(()),
        Some(now) if now.active => Err(ApiError::Forbidden),
        _ => Err(ApiError::Unauthorized),
    })
}

/// A change by an administrator in one transaction: first [`admin_holds`],
/// then `apply`; if `apply` returns an API error, nothing is stored. Taken
/// over from Roedy Mail (`api::users`).
pub(super) async fn admin_change<T: Send + 'static>(
    state: &AppState,
    admin: AuthUser,
    ip: ClientIp,
    apply: impl FnOnce(&Connection, &Actor<'_>) -> anyhow::Result<Result<T, ApiError>> + Send + 'static,
) -> Result<T, ApiError> {
    admin_change_then(state, admin, ip, apply, |_| {}).await
}

/// Like [`admin_change`]; `committed` runs right after the commit, still in
/// the database job – it also runs if the request is cancelled meanwhile
/// (such as discarding the sign-in steps of a locked account or sending its
/// notices, which it may take out of the result).
pub(super) async fn admin_change_then<T: Send + 'static>(
    state: &AppState,
    admin: AuthUser,
    ip: ClientIp,
    apply: impl FnOnce(&Connection, &Actor<'_>) -> anyhow::Result<Result<T, ApiError>> + Send + 'static,
    committed: impl FnOnce(&mut T) + Send + 'static,
) -> Result<T, ApiError> {
    db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        if let Err(error) = admin_holds(&tx, &admin)? {
            return Ok(Err(error));
        }
        let actor = Actor {
            admin: &admin,
            ip: ip.0,
        };
        let mut result = apply(&tx, &actor)?;
        if let Ok(value) = &mut result {
            tx.commit()?;
            committed(value);
        }
        Ok(result)
    })
    .await?
}

/// Changes what the invitation gives in one paired app (`role`, `None`: no
/// access), with an audit entry `apps.invite_access`; `false` if it already
/// gave that.
pub(super) fn set_invite_access(
    conn: &Connection,
    actor: &Actor<'_>,
    invite: &Invitation,
    app: &HubApp,
    role: Option<&str>,
) -> anyhow::Result<bool> {
    let Outcome::Changed { previous } = app_access::set_choice(conn, &invite.id, app, role)? else {
        return Ok(false);
    };
    let entry = Entry::new(Category::Apps, "apps.invite_access")
        .actor(actor.admin)
        .ip(actor.ip)
        .target(&invite.id)
        .details(json!({
            "slug": app.slug,
            "email": invite.email,
            "app_role": role,
            "previous": previous,
        }));
    audit_log::insert(conn, &entry)?;
    Ok(true)
}

/// The access of the form, checked against the apps: per slug the paired
/// app and the role (`None`: no access). Errors under `apps.<slug>`.
fn check_apps(
    conn: &Connection,
    apps: &BTreeMap<String, Option<String>>,
) -> rusqlite::Result<Result<Vec<(HubApp, Option<String>)>, FieldErrors>> {
    let mut errors = FieldErrors::default();
    let mut checked = Vec::with_capacity(apps.len());
    for (slug, role) in apps {
        let field = format!("apps.{slug}");
        match app_repo::by_slug(conn, slug)? {
            None => errors.code(&field, "not_found"),
            Some(app) if app.status != Status::Paired => errors.code(&field, "app_pending"),
            Some(app) => match role {
                Some(role) if !app.manifest.as_ref().is_some_and(|m| m.has_role(role)) => {
                    errors.code(&field, "invalid");
                }
                _ => checked.push((app, role.clone())),
            },
        }
    }
    Ok(if errors.is_empty() {
        Ok(checked)
    } else {
        Err(errors)
    })
}

/// The rows of invitations with the access each will give.
pub(super) fn invite_rows(
    conn: &Connection,
    invites: &[Invitation],
) -> rusqlite::Result<Vec<Value>> {
    let apps = app_repo::list(conn)?;
    let choices = app_access::all_choices(conn)?;
    Ok(invites
        .iter()
        .map(|invite| {
            invite_json(
                invite,
                &app_access::effective(&apps, choices.get(&invite.id)),
            )
        })
        .collect())
}

/// The row of one invitation; `None` if it is gone.
fn invite_row(conn: &Connection, id: &str) -> rusqlite::Result<Option<Value>> {
    let Some(invite) = invitations::get(conn, id)? else {
        return Ok(None);
    };
    Ok(invite_rows(conn, &[invite])?.pop())
}

/// List entry for an invitation: name only if it was in the form;
/// `mail_sent_at` empty = "Not sent"; `apps`: slug → role of the access it
/// will give.
pub(super) fn invite_json(invite: &Invitation, apps: &BTreeMap<String, String>) -> Value {
    json!({
        "id": invite.id,
        "status": "invited",
        "name": invite.name,
        "email": invite.email,
        "role": invite.role.as_str(),
        "created_at": invite.created_at,
        "invited_by": invite.invited_by.as_ref().map(|id| json!({
            "id": id,
            "name": invite.inviter,
        })),
        "mail_sent_at": invite.mail_sent_at,
        "expires_at": invite.expires_at,
        "expired": invite.expired,
        "requested_at": invite.requested_at,
        "apps": apps,
    })
}

/// The token from the row (for the mail and "Copy link").
fn open_token(state: &AppState, id: &str, sealed: &[u8]) -> Result<Token, ApiError> {
    let plain = state.secrets.open(TOKEN_PURPOSE, id, sealed)?;
    String::from_utf8(plain)
        .ok()
        .and_then(|text| Token::parse(&text))
        .ok_or_else(|| anyhow::anyhow!("invitation token {id} unreadable").into())
}

/// Format of invitation mails: the invited person has no settings yet, so
/// the defaults in the inviter's language apply.
async fn invite_format(state: &AppState, inviter: &AuthUser) -> anyhow::Result<Format> {
    let inviter_id = inviter.id.clone();
    let lang = db::call(&state.db, move |conn| settings::lang_of(conn, &inviter_id)).await?;
    Ok(Format::default().with_lang(lang))
}

/// Sends the mail of an invitation (valid until `expires_at`); `false` if it
/// did not go out (cause in the mailer's log).
async fn send_mail(
    state: &AppState,
    delivery: &Delivery,
    invite: &Invitation,
    token: &Token,
    inviter: &AuthUser,
    format: &Format,
    expires_at: &str,
) -> anyhow::Result<bool> {
    let until: Timestamp = expires_at.parse()?;
    let sent = delivery
        .send(state, format, |ctx| {
            Invite {
                to: &invite.email,
                name: invite.name.as_deref(),
                inviter: Some(&inviter.display_name),
                role: invite.role,
                token: token.as_str(),
                expires_at: until,
            }
            .render(ctx)
        })
        .await?;
    Ok(sent.is_ok())
}

/// The open invitations for Administration › Users.
async fn list(State(state): State<AppState>, _: AdminUser) -> Result<Json<Value>, ApiError> {
    let rows = db::call(&state.db, |conn| {
        Ok(invite_rows(conn, &invitations::list(conn)?)?)
    })
    .await?;
    Ok(Json(json!({ "invites": rows })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    emails: Vec<String>,
    role: Role,
    /// Only with a single address.
    #[serde(default)]
    name: Option<String>,
    /// Slug → role or `null` ("no access"); the other apps get their
    /// default.
    #[serde(default)]
    apps: BTreeMap<String, Option<String>>,
}

/// Validated form: addresses with their index in the input.
struct Form {
    emails: Vec<(usize, String)>,
    name: Option<String>,
    role: Role,
    apps: BTreeMap<String, Option<String>>,
}

/// Validates the form; errors per address under `emails.<i>` (index in the
/// input). Duplicate addresses count once.
fn validate(body: CreateBody) -> Result<Form, FieldErrors> {
    let mut errors = FieldErrors::default();
    if body.emails.is_empty() {
        errors.code("emails", "required");
    } else if body.emails.len() > MAX_INVITES {
        // Skip the other checks: a long list would otherwise cost time.
        errors.code("emails", "too_many");
        return Err(errors);
    }
    let mut seen = HashSet::new();
    let mut emails: Vec<(usize, String)> = Vec::new();
    for (i, email) in body.emails.iter().enumerate() {
        let email = email.trim();
        let mut one = FieldErrors::default();
        check_email(&mut one, &format!("emails.{i}"), email);
        if !one.is_empty() {
            errors.merge(one);
        } else if seen.insert(email.to_ascii_lowercase()) {
            emails.push((i, email.to_owned()));
        }
    }
    let name = body
        .name
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    if let Some(name) = &name {
        if emails.len() > 1 {
            errors.code("name", "single_only");
        } else {
            check_display_name(&mut errors, "name", name);
        }
    }
    if errors.is_empty() {
        Ok(Form {
            emails,
            name,
            role: body.role,
            apps: body.apps,
        })
    } else {
        Err(errors)
    }
}

/// "Send invitation", or without a system mail "Create invitation". Per
/// address `sent` says whether the mail went out; `recorded: false` if it
/// went out but storing that failed (the list then shows "Not sent", a
/// resend would be a second mail).
async fn create(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<CreateBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let Form {
        emails,
        name,
        role,
        apps,
    } = validate(body).map_err(FieldErrors::into_error)?;
    let delivery = Delivery::prepare(&state).await?;
    let system_mail = !matches!(delivery, Err(Unavailable::NoSystemMail));
    let mails = if delivery.is_ok() { emails.len() } else { 0 };

    // Tokens, encryption and the mail's language before the transaction.
    let mut planned = Vec::with_capacity(emails.len());
    for (i, email) in emails {
        let id = db::new_id()?;
        let token = Token::generate()?;
        let sealed = state
            .secrets
            .seal(TOKEN_PURPOSE, &id, token.as_str().as_bytes())?;
        planned.push((i, email, id, token, sealed));
    }
    let format = invite_format(&state, &admin).await?;

    // Creating and sending in one separate task that also runs to completion
    // when the client aborts – otherwise invitations could be stored without
    // their mails, or a sent one could show up as "Not sent".
    let worker = state.clone();
    let rows = tokio::spawn(async move {
        // One attempt per mail, owned by the worker: each mail it sends
        // counts, the rest are returned when it ends – also when the request
        // is cancelled, the limit applies to every mail that goes out.
        let mut reserved = (0..mails)
            .map(|_| {
                worker
                    .limiter
                    .reserve(&INVITE_MAIL, Key::User(admin.id.clone()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let created = admin_change(&worker, admin.clone(), ip, move |tx, actor| {
            let mut taken = FieldErrors::default();
            for (i, email, ..) in &planned {
                if users::email_taken(tx, email, "", false)? || invitations::exists_for(tx, email)?
                {
                    taken.code(&format!("emails.{i}"), "taken");
                }
            }
            let chosen = match check_apps(tx, &apps)? {
                Ok(chosen) => chosen,
                Err(errors) => {
                    taken.merge(errors);
                    Vec::new()
                }
            };
            if !taken.is_empty() {
                return Ok(Err(taken.into_error()));
            }
            let all_apps = app_repo::list(tx)?;
            let mut created = Vec::with_capacity(planned.len());
            for (_, email, id, token, sealed) in planned {
                let invite = invitations::insert(
                    tx,
                    &NewInvitation {
                        id: &id,
                        email: &email,
                        name: name.as_deref(),
                        role,
                        token_hash: &token.hash(),
                        token_enc: &sealed,
                        invited_by: &actor.admin.id,
                    },
                )?;
                for (app, app_role) in &chosen {
                    app_access::set_choice(tx, &invite.id, app, app_role.as_deref())?;
                }
                // Open changes of other accounts to this address end for
                // good, also if the invitation is revoked later.
                email_changes::discard_to(tx, &invite.email)?;
                let gives =
                    app_access::effective(&all_apps, Some(&app_access::choices(tx, &invite.id)?));
                let details = json!({
                    "email": invite.email,
                    "role": role.as_str(),
                    "apps": gives,
                });
                audit_log::insert(tx, &actor.entry("users.invite", &invite.id, details))?;
                created.push((invite, token));
            }
            Ok(Ok(created))
        })
        .await?;
        let mut rows = Vec::with_capacity(created.len());
        for (invite, token) in &created {
            let mut row = json!({ "id": invite.id, "email": invite.email, "sent": false });
            let delivery = match &delivery {
                Ok(delivery) => delivery,
                Err(why) => {
                    tracing::info!(?why, "invitation only created, no mail");
                    rows.push(row);
                    continue;
                }
            };
            let until = &invite.expires_at;
            match send_mail(&worker, delivery, invite, token, &admin, &format, until).await {
                Ok(true) => {}
                Ok(false) => {
                    rows.push(row);
                    continue;
                }
                Err(error) => {
                    tracing::warn!("invitation not sent: {error:#}");
                    rows.push(row);
                    continue;
                }
            }
            row["sent"] = true.into();
            if let Some(attempt) = reserved.pop() {
                attempt.count();
            }
            let (id, by, until) = (invite.id.clone(), admin.id.clone(), until.clone());
            let recorded = db::call(&worker.db, move |conn| {
                Ok(invitations::mark_sent(conn, &id, &by, &until)?)
            })
            .await;
            if let Err(error) = &recorded {
                tracing::warn!("invitation sent, but not recorded: {error:#}");
            }
            if !matches!(recorded, Ok(true)) {
                row["recorded"] = false.into();
            }
            rows.push(row);
        }
        Ok::<_, ApiError>(rows)
    })
    .await
    .map_err(|error| anyhow::anyhow!("sending aborted: {error}"))??;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "invites": rows, "system_mail": system_mail })),
    ))
}

/// "Resend": the same link, one already passed on stays valid.
async fn resend(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let delivery = Delivery::prepare(&state).await?.map_err(|why| {
        tracing::warn!(?why, "resend invitation: no system mail");
        mail_failed()
    })?;
    let (lookup, holder) = (id.clone(), admin.clone());
    let found = db::call(&state.db, move |conn| {
        if let Err(error) = admin_holds(conn, &holder)? {
            return Ok(Err(error));
        }
        let Some(invite) = invitations::get(conn, &lookup)? else {
            return Ok(Err(ApiError::NotFound));
        };
        let sealed = invitations::token_enc(conn, &lookup)?.unwrap_or_default();
        Ok(Ok((invite, sealed, invitations::next_expiry(conn)?)))
    })
    .await??;
    let (invite, sealed, until) = found;
    let token = open_token(&state, &invite.id, &sealed)?;
    state
        .limiter
        .acquire(&INVITE_MAIL, Key::User(admin.id.clone()))?;

    // Sending and recording in a separate task that also runs to completion
    // when the client aborts.
    let format = invite_format(&state, &admin).await?;
    let worker = state.clone();
    tokio::spawn(async move {
        if !send_mail(&worker, &delivery, &invite, &token, &admin, &format, &until).await? {
            return Err(mail_failed());
        }
        // The mail went out and is recorded, even if the administrator has
        // been locked or lost the role in the meantime – they then do not
        // become the inviter (`invitations::mark_sent`).
        let id = invite.id.clone();
        db::call(&worker.db, move |conn| {
            let tx = conn.transaction()?;
            // Revoked or accepted in the meantime: the link is not valid.
            if !invitations::mark_sent(&tx, &id, &admin.id, &until)? {
                return Ok(Err(ApiError::NotFound));
            }
            let actor = Actor {
                admin: &admin,
                ip: ip.0,
            };
            let details = json!({ "email": invite.email });
            audit_log::insert(&tx, &actor.entry("users.invite_resend", &id, details))?;
            let now = invite_row(&tx, &id)?;
            tx.commit()?;
            Ok(Ok(Json(json!({ "invite": now }))))
        })
        .await?
    })
    .await
    .map_err(|error| anyhow::anyhow!("sending aborted: {error}"))?
}

/// "Copy link": the link from the mail, even if it never went out.
async fn link(
    State(state): State<AppState>,
    _: AdminUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let lookup = id.clone();
    let sealed = db::call(&state.db, move |conn| {
        Ok(invitations::token_enc(conn, &lookup)?)
    })
    .await?
    .ok_or(ApiError::NotFound)?;
    let token = open_token(&state, &id, &sealed)?;
    let url = format!(
        "{}/invite/{}",
        state.public_url.origin().ascii_serialization(),
        token.as_str()
    );
    Ok(Json(json!({ "url": url })))
}

/// "Extend": valid for seven days again, without a mail; the link stays the
/// same, one already passed on works again.
async fn renew(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let now = one_invite(&state, admin, ip, id, |conn, actor, invite| {
        let until = invitations::next_expiry(conn)?;
        if !invitations::renew(conn, &invite.id, &actor.admin.id, &until)? {
            return Ok(Ok(()));
        }
        let details = json!({ "email": invite.email });
        audit_log::insert(
            conn,
            &actor.entry("users.invite_renew", &invite.id, details),
        )?;
        Ok(Ok(()))
    })
    .await?;
    Ok(Json(json!({ "invite": now })))
}

/// Changes an invitation (`admin_change`); then returns its row (`None`
/// after revoking). `apply` may refuse with an API error.
async fn one_invite(
    state: &AppState,
    admin: AuthUser,
    ip: ClientIp,
    id: String,
    apply: impl FnOnce(&Connection, &Actor<'_>, &Invitation) -> anyhow::Result<Result<(), ApiError>>
    + Send
    + 'static,
) -> Result<Option<Value>, ApiError> {
    admin_change(state, admin, ip, move |tx, actor| {
        let Some(invite) = invitations::get(tx, &id)? else {
            return Ok(Err(ApiError::NotFound));
        };
        if let Err(error) = apply(tx, actor, &invite)? {
            return Ok(Err(error));
        }
        Ok(Ok(invite_row(tx, &id)?))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    #[serde(default)]
    role: Option<Role>,
    /// Only the apps named change.
    #[serde(default)]
    apps: BTreeMap<String, Option<String>>,
}

/// "Role" and "Access" of an invitation.
async fn update(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<UpdateBody>,
) -> Result<Json<Value>, ApiError> {
    let now = one_invite(&state, admin, ip, id, move |conn, actor, invite| {
        let chosen = match check_apps(conn, &body.apps)? {
            Ok(chosen) => chosen,
            Err(errors) => return Ok(Err(errors.into_error())),
        };
        if let Some(role) = body.role {
            change_role(conn, actor, invite, role)?;
        }
        for (app, app_role) in &chosen {
            set_invite_access(conn, actor, invite, app, app_role.as_deref())?;
        }
        Ok(Ok(()))
    })
    .await?;
    Ok(Json(json!({ "invite": now })))
}

/// "Revoke invitation" after confirmation: the link no longer works.
async fn revoke(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    one_invite(&state, admin, ip, id, |conn, actor, invite| {
        revoke_invite(conn, actor, invite)?;
        Ok(Ok(()))
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Invitation for the token from the link; unknown tokens count towards the
/// per-IP limit (like the other links from mails) and are 404.
async fn find(state: &AppState, ip: ClientIp, token: &str) -> Result<([u8; 32], Found), ApiError> {
    let attempt = state.limiter.reserve(&LINK_IP, Key::Ip(ip.network()))?;
    let Some(token) = Token::parse(token) else {
        attempt.count();
        return Err(ApiError::NotFound);
    };
    let hash = token.hash();
    let found = db::call(&state.db, move |conn| {
        Ok(invitations::by_token(conn, &hash)?)
    })
    .await?;
    match found {
        Some(found) => Ok((hash, found)),
        None => {
            attempt.count();
            Err(ApiError::NotFound)
        }
    }
}

/// Invitation page: details for "Welcome" or "Invitation expired". Names in
/// display form ("Max Example"). Like changing requests only with
/// `X-Requested-With`: unknown tokens count towards the per-IP limit, and a
/// foreign `<img>` should not be able to trigger them.
async fn show(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<Json<Value>, ApiError> {
    super::require_requested_with(&headers)?;
    let (_, found) = find(&state, ip, &token).await?;
    let invite = found.invitation;
    Ok(Json(json!({
        "email": invite.email,
        "name": invite.name.as_deref().map(full_name),
        "role": invite.role.as_str(),
        "inviter": invite.inviter.as_deref().map(full_name),
        "expires_at": invite.expires_at,
        "expired": invite.expired,
    })))
}

/// "Multi-factor" step: new secret for the authenticator app (an earlier one
/// of the invitation then no longer applies).
async fn enroll(
    State(state): State<AppState>,
    ip: ClientIp,
    Path(token): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (_, found) = find(&state, ip, &token).await?;
    let invite = found.invitation;
    if invite.expired {
        return Err(expired());
    }
    let secret = totp::generate_secret()?;
    let sealed = state.secrets.seal(TOTP_PURPOSE, &invite.id, &secret)?;
    let (base32, uri) = totp::enrollment(&secret, &state.issuer(), &invite.email)?;
    let id = invite.id;
    let stored = db::call(&state.db, move |conn| {
        Ok(invitations::set_totp(conn, &id, &sealed)?)
    })
    .await?;
    if !stored {
        return Err(expired());
    }
    Ok(Json(json!({ "secret": base32, "uri": uri })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptBody {
    name: String,
    password: String,
    /// Code from the app of the "Multi-factor" step; without it the account
    /// signs in with the password only.
    #[serde(default)]
    code: Option<String>,
}

/// The authenticator app from the "Multi-factor" step, checked.
struct App {
    /// The secret as stored in the invitation (checked again in the
    /// transaction).
    sealed: Vec<u8>,
    device_id: String,
    device_secret: Vec<u8>,
    step: u64,
}

/// Result of the transaction in [`accept`].
struct Accepted {
    user: AuthUser,
    cookie: HeaderValue,
    codes: Option<Vec<String>>,
}

/// "Activate account": name, password and optionally the code from the app;
/// signed in afterwards.
async fn accept(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    previous: Option<CurrentUser>,
    Path(token): Path<String>,
    JsonBody(body): JsonBody<AcceptBody>,
) -> Result<Response, ApiError> {
    let (hash, found) = find(&state, ip, &token).await?;
    let invite = found.invitation;
    if invite.expired {
        return Err(expired());
    }
    let name = body.name.trim().to_owned();
    let code = body
        .code
        .as_deref()
        .map(totp::compact)
        .filter(|code| !code.is_empty());
    let mut errors = FieldErrors::default();
    check_display_name(&mut errors, "name", &name);
    check_new_password(&mut errors, "password", &body.password);
    if code
        .as_deref()
        .is_some_and(|code| !totp::is_well_formed(code))
    {
        errors.code("code", "invalid");
    }
    if !errors.is_empty() {
        return Err(errors.into_error());
    }

    // The code before the hash: a wrong one costs no computing time.
    let app = match code {
        Some(code) => {
            let Some(sealed) = found.totp_secret_enc else {
                return Err(rejected("totp_required"));
            };
            let attempt = state
                .limiter
                .reserve(&INVITE_CODE, Key::User(format!("invite:{}", invite.id)))?;
            let secret = state.secrets.open(TOTP_PURPOSE, &invite.id, &sealed)?;
            let Some(step) = totp::matching_step(&secret, &code, totp::now())? else {
                attempt.count();
                return Err(auth::invalid_code());
            };
            drop(attempt);
            let device_id = db::new_id()?;
            let device_secret = state
                .secrets
                .seal(totp::SECRET_PURPOSE, &device_id, &secret)?;
            Some(App {
                sealed,
                device_id,
                device_secret,
                step,
            })
        }
        None => None,
    };
    let password_hash = state.hasher.hash(&body.password).await?;
    let origin = auth::origin(&state, &headers, ip);
    let previous = previous.map(|CurrentUser(user)| user);
    let previous_sid = previous.as_ref().map(|user| user.session_id.clone());

    let accepted = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        // Accepted, revoked, expired or a new secret created in the meantime:
        // the check applied to a different state.
        let Some(now) = invitations::by_token(&tx, &hash)? else {
            return Ok(Err(ApiError::NotFound));
        };
        if now.invitation.expired {
            return Ok(Err(expired()));
        }
        if let Some(app) = &app
            && now.totp_secret_enc.as_deref() != Some(app.sealed.as_slice())
        {
            return Ok(Err(auth::invalid_code()));
        }
        let invite = now.invitation;
        let username = users::unique_username(&tx, &invite.email)?;
        // The access the invitation chose, the defaults for the other apps;
        // before the invitation (and its choices) go.
        let user_id = users::insert_with(
            &tx,
            &NewUser {
                username: &username,
                email: &invite.email,
                display_name: &name,
                role: invite.role,
                password_hash: &password_hash,
            },
            NewAccess::Invitation(&invite.id),
        )?;
        let granted = app_access::of_user(&tx, &user_id)?;
        let codes = match &app {
            Some(app) => {
                // No settings yet: the app name in the default language.
                let app_name = totp::default_name(&[], settings::Language::default());
                totp_devices::insert_confirmed(
                    &tx,
                    &app.device_id,
                    &user_id,
                    &app_name,
                    &app.device_secret,
                    app.step,
                )?;
                Some(recovery::replace(&tx, &user_id)?)
            }
            None => None,
        };
        invitations::delete(&tx, &invite.id)?;
        // Like a sign-in: an earlier session of the browser ends.
        if let Some(previous) = &previous {
            sessions::delete(&tx, &previous.id, &previous.session_id, Reason::Logout)?;
        }
        let client = origin.client();
        let started = session::start(&tx, &user_id, &client, true)?;
        // The account's first device: known, without a notice.
        known_devices::remember(&tx, &user_id, &origin.device_hash)?;
        let user = AuthUser {
            id: user_id,
            username,
            email: invite.email.clone(),
            display_name: name.clone(),
            role: invite.role,
            session_id: started.id.clone(),
            auth_time: started.auth_time,
        };
        audit_log::insert(
            &tx,
            &Entry::new(Category::Users, "users.invite_accept")
                .actor(&user)
                .ip(origin.ip)
                .target(&user.id)
                .details(json!({
                    "email": user.email,
                    "role": user.role.as_str(),
                    "invitation": invite.id,
                    "invited_by": invite.invited_by,
                    "mfa": app.is_some(),
                    "apps": granted,
                })),
        )?;
        audit_log::insert(
            &tx,
            &Entry::new(Category::Login, "auth.login")
                .actor(&user)
                .ip(origin.ip)
                .target(&started.id)
                .details(json!({ "method": "invite" })),
        )?;
        tx.commit()?;
        Ok(Ok(Accepted {
            user,
            cookie: started.cookie,
            codes,
        }))
    })
    .await??;
    if let Some(previous) = previous_sid {
        state.end_session(&previous);
    }
    tracing::info!(user = %accepted.user.id, "invitation accepted");
    let body = Json(json!({
        "user": auth::user_json(&accepted.user),
        "recovery_codes": accepted.codes,
    }));
    Ok((
        StatusCode::CREATED,
        [(header::SET_COOKIE, accepted.cookie)],
        body,
    )
        .into_response())
}

/// "Request a new invitation" on the page of an expired invitation: always
/// 202; it is recorded at most once per 24 hours. Without an account the
/// audit log names the invited address as the actor.
async fn request(
    State(state): State<AppState>,
    ip: ClientIp,
    Path(token): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (_, found) = find(&state, ip, &token).await?;
    let invite = found.invitation;
    if !invite.expired {
        return Err(rejected("invite_valid"));
    }
    db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        if invitations::request(&tx, &invite.id)? {
            let mut entry = Entry::new(Category::Users, "users.invite_request")
                .ip(ip.0)
                .target(&invite.id)
                .details(json!({ "email": invite.email }));
            entry.actor_name = Some(invite.email.clone());
            audit_log::insert(&tx, &entry)?;
        }
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(value: Value) -> CreateBody {
        serde_json::from_value(value).unwrap()
    }

    fn errors_of(value: Value) -> Value {
        validate(body(value)).err().unwrap().into_value()
    }

    #[test]
    fn validates_addresses_and_the_name() {
        let form = validate(body(json!({
            "emails": [" jonas@example.com ", "JONAS@example.com", "eva@example.com"],
            "role": "user",
        })))
        .ok()
        .unwrap();
        // Duplicates count once, ignoring case.
        assert_eq!(
            form.emails,
            [
                (0, "jonas@example.com".to_owned()),
                (2, "eva@example.com".to_owned())
            ]
        );
        assert_eq!(
            errors_of(json!({ "emails": [], "role": "admin" })),
            json!({ "emails": { "code": "required" } })
        );
        assert_eq!(
            errors_of(json!({ "emails": ["ok@example.com", "nope"], "role": "user" })),
            json!({ "emails.1": { "code": "invalid" } })
        );
        assert_eq!(
            errors_of(json!({
                "emails": ["a@example.com", "b@example.com"],
                "role": "user",
                "name": "Example, Jonas",
            })),
            json!({ "name": { "code": "single_only" } })
        );
        let many: Vec<String> = (0..=MAX_INVITES)
            .map(|i| format!("u{i}@example.com"))
            .collect();
        assert_eq!(
            errors_of(json!({ "emails": many, "role": "user" })),
            json!({ "emails": { "code": "too_many" } })
        );
        assert!(
            serde_json::from_value::<CreateBody>(json!({ "emails": [], "role": "owner" })).is_err()
        );
    }
}
