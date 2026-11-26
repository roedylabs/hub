//! Administration › Users.
//!
//! - `GET users`: active, locked and invited users in one list (`status`),
//!   with `mfa` and `last_seen_at`.
//! - `GET users/:id`: one user's row plus `sessions` (signed-in devices) and
//!   `recovery_codes` (unused and total, `null` without codes).
//! - `PATCH users/:id` (`{role}`), `POST users/:id/lock|unlock`,
//!   `POST users/:id/reset-mfa`, `DELETE users/:id`: not for the own account
//!   (422 `own_account`); invitations via `invites/:id`.
//! - `POST users/bulk` (`{action, ids, role?}`): role, lock, unlock or remove
//!   for the selection of the list – users and invitations mixed; whatever
//!   does not fit (such as "lock" for a locked account or the own one) stays
//!   as it is. The response names the changed IDs.
//!
//! Every change checks in its transaction whether the acting administrator is
//! still signed in and an administrator (`invites::admin_holds`): otherwise
//! two admins taking each other's role at the same time would leave nobody.
//! The sole active administrator is never locked, demoted, reset or removed
//! (409 `last_admin`; in bulk actions skipped) – with the rule for the own
//! account this cannot happen today, the check guards the invariant in the
//! same transaction anyway. Locking ends all sessions, discards open links
//! from mails and the person's pending sign-in steps. "Reset second factor"
//! removes authenticator apps and recovery codes like `reset-admin`, without
//! a new password and without ending the sessions, and sends the person the
//! security notice `security_mfa_reset`.
//!
//! Taken over from Roedy Mail (`crates/server/src/api/users.rs`) without the
//! notifications for the administrators and the mail accounts.

use std::collections::BTreeSet;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    ApiError, FieldErrors, JsonBody,
    invites::{self, Actor, admin_change_then, invite_json},
    mfa::mfa_disabled,
};
use crate::{
    app::AppState,
    auth::{AdminUser, AuthUser},
    client_ip::ClientIp,
    db::{
        self,
        repo::{
            self, audit_log, invitations, recovery_codes, sessions, totp_devices,
            users::{self, Listed, Profile, Role},
        },
    },
    notices::{self, Outbox, Security},
};

/// At most this many IDs per bulk action.
const MAX_BULK: usize = 500;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/users", get(list))
        .route("/users/bulk", post(bulk))
        .route("/users/{id}", get(show).delete(remove).patch(update))
        .route("/users/{id}/lock", post(lock))
        .route("/users/{id}/unlock", post(unlock))
        .route("/users/{id}/reset-mfa", post(reset_mfa))
}

/// 422 `own_account`: the own account is changed under Account.
fn own_account() -> ApiError {
    ApiError::rejected(StatusCode::UNPROCESSABLE_ENTITY, "own_account")
}

/// 409 `last_admin`: the change would leave no active administrator.
fn last_admin() -> ApiError {
    ApiError::rejected(StatusCode::CONFLICT, "last_admin")
}

/// List entry for a user.
fn user_json(user: &Listed) -> Value {
    let p = &user.profile;
    json!({
        "id": p.id,
        "status": if user.active { "active" } else { "locked" },
        "name": p.display_name,
        "email": p.email,
        "username": p.username,
        "role": p.role.as_str(),
        "mfa": user.mfa,
        "last_seen_at": user.last_seen_at,
        "created_at": user.created_at,
    })
}

async fn list(State(state): State<AppState>, _: AdminUser) -> Result<Json<Value>, ApiError> {
    let (users, invites) = db::call(&state.db, |conn| {
        Ok((users::list(conn)?, invitations::list(conn)?))
    })
    .await?;
    let rows: Vec<Value> = users
        .iter()
        .map(user_json)
        .chain(invites.iter().map(invite_json))
        .collect();
    Ok(Json(json!({ "users": rows })))
}

/// The expanded row: signed-in devices (without IP addresses, which only the
/// person sees) and the state of the recovery codes.
async fn show(
    State(state): State<AppState>,
    _: AdminUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let found = db::call(&state.db, move |conn| {
        let Some(row) = users::get_listed(conn, &id)? else {
            return Ok(None);
        };
        let devices: Vec<Value> = sessions::list(conn, &id)?
            .into_iter()
            .map(|s| json!({ "created_at": s.created_at, "last_seen_at": s.last_seen_at }))
            .collect();
        let codes = recovery_codes::summary(conn, &id)?
            .map(|c| json!({ "unused": c.unused, "total": c.total }));
        Ok(Some((row, devices, codes)))
    })
    .await?;
    let (row, devices, codes) = found.ok_or(ApiError::NotFound)?;
    let mut body = user_json(&row);
    body["sessions"] = json!(devices);
    body["recovery_codes"] = json!(codes);
    Ok(Json(json!({ "user": body })))
}

fn names(user: &Profile) -> Value {
    json!({ "name": user.display_name, "email": user.email })
}

/// Whether `user` is the sole active administrator, whom no change may take
/// away ([`last_admin`]).
fn is_last_admin(conn: &Connection, user: &Profile) -> rusqlite::Result<bool> {
    users::sole_admin(conn, &user.id)
}

/// New role with audit log entry; `false` if it already applied or it
/// would take the last administrator.
fn change_role(
    conn: &Connection,
    actor: &Actor<'_>,
    user: &Profile,
    role: Role,
) -> anyhow::Result<bool> {
    if role != Role::Admin && is_last_admin(conn, user)? {
        return Ok(false);
    }
    if !users::set_role(conn, &user.id, role)? {
        return Ok(false);
    }
    let mut details = names(user);
    details["role"] = json!(role.as_str());
    details["previous"] = json!(user.role.as_str());
    audit_log::insert(conn, &actor.entry("users.role", &user.id, details))?;
    Ok(true)
}

/// Locks: sessions end, open links from mails no longer work; `false` if the
/// account was already locked or is the last administrator.
fn lock_user(conn: &Connection, actor: &Actor<'_>, user: &Profile) -> anyhow::Result<bool> {
    if is_last_admin(conn, user)? || !users::lock(conn, &user.id)? {
        return Ok(false);
    }
    let ended = sessions::delete_for_user(conn, &user.id, None)?;
    repo::discard_links(conn, &user.id)?;
    let mut details = names(user);
    details["sessions_ended"] = json!(ended);
    audit_log::insert(conn, &actor.entry("users.lock", &user.id, details))?;
    Ok(true)
}

fn unlock_user(conn: &Connection, actor: &Actor<'_>, user: &Profile) -> anyhow::Result<bool> {
    if !users::unlock(conn, &user.id)? {
        return Ok(false);
    }
    audit_log::insert(conn, &actor.entry("users.unlock", &user.id, names(user)))?;
    Ok(true)
}

/// Removes with everything attached to the account; `false` if it is gone
/// already or is the last administrator.
fn remove_user(conn: &Connection, actor: &Actor<'_>, user: &Profile) -> anyhow::Result<bool> {
    if is_last_admin(conn, user)? || !users::delete(conn, &user.id)? {
        return Ok(false);
    }
    let mut details = names(user);
    details["role"] = json!(user.role.as_str());
    audit_log::insert(conn, &actor.entry("users.remove", &user.id, details))?;
    Ok(true)
}

/// What a change to one user does.
enum Change {
    Role(Role),
    Lock,
    Unlock,
    Remove,
}

impl Change {
    /// Whether the change takes access or the role away – never from the last
    /// administrator.
    fn takes_away(&self) -> bool {
        matches!(self, Self::Role(Role::User) | Self::Lock | Self::Remove)
    }

    fn apply(&self, conn: &Connection, actor: &Actor<'_>, user: &Profile) -> anyhow::Result<bool> {
        match *self {
            Self::Role(role) => change_role(conn, actor, user, role),
            Self::Lock => lock_user(conn, actor, user),
            Self::Unlock => unlock_user(conn, actor, user),
            Self::Remove => remove_user(conn, actor, user),
        }
    }
}

/// Common flow of the endpoints for one user: not the own account,
/// `admin_change`, the last-admin rule, then the row of the list (`None`
/// after removal). Locked and removed accounts lose their pending sign-in
/// steps right after the commit, even if the request is cancelled.
async fn one_user(
    state: &AppState,
    admin: AuthUser,
    ip: ClientIp,
    id: String,
    change: Change,
) -> Result<Option<Value>, ApiError> {
    if id == admin.id {
        return Err(own_account());
    }
    let target = id.clone();
    let challenges = state.challenges.clone();
    admin_change_then(
        state,
        admin,
        ip,
        move |tx, actor| {
            let Some(user) = users::credentials_by_id(tx, &target)?.map(|c| c.user) else {
                return Ok(Err(ApiError::NotFound));
            };
            if change.takes_away() && is_last_admin(tx, &user)? {
                return Ok(Err(last_admin()));
            }
            // Like `bulk`: a lock or removal that took place ends the
            // pending sign-in steps.
            let done = change.apply(tx, actor, &user)?;
            let ends_sign_ins = done && matches!(change, Change::Lock | Change::Remove);
            let row = users::get_listed(tx, &target)?.map(|row| user_json(&row));
            Ok(Ok((row, ends_sign_ins)))
        },
        move |(_, ends_sign_ins)| {
            if *ends_sign_ins {
                challenges.discard_user(&id);
            }
        },
    )
    .await
    .map(|(row, _)| row)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RoleBody {
    pub role: Role,
}

/// "Role" in the expanded row.
async fn update(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<RoleBody>,
) -> Result<Json<Value>, ApiError> {
    let row = one_user(&state, admin, ip, id, Change::Role(body.role)).await?;
    Ok(Json(json!({ "user": row })))
}

async fn lock(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let row = one_user(&state, admin, ip, id, Change::Lock).await?;
    Ok(Json(json!({ "user": row })))
}

async fn unlock(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let row = one_user(&state, admin, ip, id, Change::Unlock).await?;
    Ok(Json(json!({ "user": row })))
}

/// "Remove" after confirmation.
async fn remove(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    one_user(&state, admin, ip, id, Change::Remove).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// "Reset second factor": also for administrators and locked accounts, not
/// for the own one or the last administrator, and without a confirmed app
/// 409 `mfa_disabled`. The mail is not part of the transaction: the reset
/// applies even without system mail or on a sending error.
async fn reset_mfa(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if id == admin.id {
        return Err(own_account());
    }
    let target = id.clone();
    let (challenges, sender) = (state.challenges.clone(), state.clone());
    let (row, _) = admin_change_then(
        &state,
        admin,
        ip,
        move |tx, actor| {
            let Some(user) = users::credentials_by_id(tx, &target)?.map(|c| c.user) else {
                return Ok(Err(ApiError::NotFound));
            };
            if is_last_admin(tx, &user)? {
                return Ok(Err(last_admin()));
            }
            if !totp_devices::has_confirmed(tx, &user.id)? {
                return Ok(Err(mfa_disabled()));
            }
            let devices = totp_devices::delete_all(tx, &user.id)?;
            recovery_codes::delete_all(tx, &user.id)?;
            let mut outbox = Outbox::default();
            let event = Security::MfaReset {
                by: &actor.admin.display_name,
            };
            notices::security(tx, &mut outbox, (&user).into(), event)?;
            let mut details = names(&user);
            details["devices"] = json!(devices);
            audit_log::insert(tx, &actor.entry("users.mfa_reset", &user.id, details))?;
            let row = users::get_listed(tx, &user.id)?.map(|row| user_json(&row));
            Ok(Ok((row, outbox)))
        },
        move |(_, outbox)| {
            // Otherwise a pending sign-in step would apply again as soon as the
            // person confirms a new app within the five minutes.
            challenges.discard_user(&id);
            std::mem::take(outbox).send(&sender);
        },
    )
    .await?;
    Ok(Json(json!({ "user": row })))
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Action {
    Role,
    Lock,
    Unlock,
    Remove,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BulkBody {
    action: Action,
    ids: Vec<String>,
    #[serde(default)]
    role: Option<Role>,
}

/// Bulk actions on the selection: role for users and invitations, lock only
/// active ones, unlock only locked ones, remove deletes users and withdraws
/// invitations – all in one transaction. The own account and the last
/// administrator, like everything that does not fit, stay unchanged (the
/// list does not let the own account be selected anyway).
async fn bulk(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<BulkBody>,
) -> Result<Json<Value>, ApiError> {
    let mut errors = FieldErrors::default();
    if body.ids.is_empty() {
        errors.code("ids", "required");
    } else if body.ids.len() > MAX_BULK {
        errors.code("ids", "too_many");
    }
    let role = match (body.action, body.role) {
        (Action::Role, None) => {
            errors.code("role", "required");
            None
        }
        (_, role) => role,
    };
    if !errors.is_empty() {
        return Err(errors.into_error());
    }
    let mut ids: BTreeSet<String> = body.ids.into_iter().collect();
    ids.remove(&admin.id);
    let action = body.action;
    let challenges = state.challenges.clone();
    let (changed, _) = admin_change_then(
        &state,
        admin,
        ip,
        move |tx, actor| {
            let (mut changed, mut gone) = (Vec::new(), Vec::new());
            for id in &ids {
                let done = if let Some(user) = users::credentials_by_id(tx, id)?.map(|c| c.user) {
                    let change = match action {
                        Action::Role => Change::Role(role.unwrap_or(user.role)),
                        Action::Lock => Change::Lock,
                        Action::Unlock => Change::Unlock,
                        Action::Remove => Change::Remove,
                    };
                    let done = change.apply(tx, actor, &user)?;
                    if done && matches!(action, Action::Lock | Action::Remove) {
                        gone.push(id.clone());
                    }
                    done
                } else if let Some(invite) = invitations::get(tx, id)? {
                    match action {
                        Action::Role => {
                            invites::change_role(tx, actor, &invite, role.unwrap_or(invite.role))?
                        }
                        Action::Remove => invites::revoke_invite(tx, actor, &invite)?,
                        Action::Lock | Action::Unlock => false,
                    }
                } else {
                    false
                };
                if done {
                    changed.push(id.clone());
                }
            }
            Ok(Ok((changed, gone)))
        },
        move |(_, gone)| {
            for id in gone.iter() {
                challenges.discard_user(id);
            }
        },
    )
    .await?;
    Ok(Json(json!({ "changed": changed })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::AuthUser,
        db::repo::users::tests::{add, conn},
    };

    fn profile(conn: &Connection, id: &str) -> Profile {
        users::credentials_by_id(conn, id).unwrap().unwrap().user
    }

    /// With the own-account rule the API never reaches the last
    /// administrator; the changes themselves refuse anyway.
    #[test]
    fn the_last_admin_keeps_access_and_role() {
        let conn = conn();
        let anna = add(&conn, "anna", "anna@example.com", Role::Admin);
        let eva = add(&conn, "eva", "eva@example.com", Role::Admin);
        let admin = AuthUser::new(profile(&conn, &eva), "s".into(), 0);
        let actor = Actor {
            admin: &admin,
            ip: "198.51.100.7".parse().unwrap(),
        };
        // Eva is locked: Anna is the only active administrator.
        users::lock(&conn, &eva).unwrap();
        let target = profile(&conn, &anna);
        assert!(is_last_admin(&conn, &target).unwrap());
        for change in [Change::Lock, Change::Role(Role::User), Change::Remove] {
            assert!(change.takes_away());
            assert!(!change.apply(&conn, &actor, &target).unwrap());
        }
        let after = users::credentials_by_id(&conn, &anna).unwrap().unwrap();
        assert!(after.active);
        assert_eq!(after.user.role, Role::Admin);
        // Unlocking and keeping the role are fine.
        assert!(!Change::Unlock.takes_away() && !Change::Role(Role::Admin).takes_away());
        let count: i64 = conn
            .query_row("SELECT count(*) FROM audit_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);

        // With a second active administrator Anna can be locked.
        users::unlock(&conn, &eva).unwrap();
        assert!(!is_last_admin(&conn, &target).unwrap());
        assert!(Change::Lock.apply(&conn, &actor, &target).unwrap());
    }
}
