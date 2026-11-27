//! Tables `app_access` and `invitation_access`: which user may use which
//! app, with which role from the app's manifest (docs/app-contract.md ›
//! Access and roles), and what an invitation will give. No row in
//! `app_access`, no access. Every change gives the app `access.changed`
//! (`events`) in the same transaction.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, OptionalExtension as _};

use roedy_hub_contract::Manifest;

use crate::db::{
    self,
    repo::{
        apps::{self, AccessDefault, App, Status},
        events,
        users::{self, Profile},
    },
};

/// Runs an `INSERT`/`UPDATE … RETURNING user_id, app_id, app_role` and
/// reports every row to its app (`access.changed`, ordered by app and
/// user); returns how many.
fn changing(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<usize> {
    db::atomic(conn, |conn| {
        let mut changed = conn
            .prepare(sql)?
            .query_map(params, |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        changed.sort_by(|a, b| (&a.1, &a.0).cmp(&(&b.1, &b.0)));
        for (user_id, app_id, role) in &changed {
            events::access_changed(conn, app_id, user_id, Some(role))?;
        }
        Ok(changed.len())
    })
}

const CHANGED: &str = "RETURNING user_id, app_id, app_role";

/// The defaults after pairing (contract § 11): every hub administrator gets
/// the app's admin role (or the default role without one); with "everyone"
/// all other users get the default role. Existing entries stay as they are
/// (pairing again). Returns the number of new entries.
pub fn grant_after_pairing(
    conn: &Connection,
    app_id: &str,
    manifest: &Manifest,
) -> rusqlite::Result<usize> {
    let (Some(admin_role), Some(default_role)) =
        (manifest.role_for_hub_admins(), manifest.default_role())
    else {
        return Ok(0);
    };
    db::atomic(conn, |conn| {
        let admins = changing(
            conn,
            &format!(
                "INSERT OR IGNORE INTO app_access (user_id, app_id, app_role) \
                 SELECT id, ?1, ?2 FROM users WHERE role = 'admin' {CHANGED}"
            ),
            (app_id, &admin_role.id),
        )?;
        let others = changing(
            conn,
            &format!(
                "INSERT OR IGNORE INTO app_access (user_id, app_id, app_role) \
                 SELECT u.id, a.id, ?2 FROM users u, apps a \
                 WHERE a.id = ?1 AND a.access_default = 'all' {CHANGED}"
            ),
            (app_id, &default_role.id),
        )?;
        Ok(admins + others)
    })
}

/// Access for a new user: the default role of every paired app whose
/// default is "everyone".
pub fn grant_to_new_user(conn: &Connection, user_id: &str) -> rusqlite::Result<usize> {
    changing(
        conn,
        &format!(
            "INSERT OR IGNORE INTO app_access (user_id, app_id, app_role) \
             SELECT ?1, a.id, r.value ->> 'id' \
             FROM apps a, json_each(a.manifest_json, '$.roles') r \
             WHERE a.state = 'paired' AND a.access_default = 'all' \
               AND r.value ->> 'default' = 1 {CHANGED}"
        ),
        [user_id],
    )
}

/// Roles the manifest no longer has fall back to its default role (contract
/// § 8 › Changed roles). Returns the roles that were gone, sorted, and the
/// number of users who had them.
pub fn fall_back_to_default(
    conn: &Connection,
    app_id: &str,
    manifest: &Manifest,
) -> rusqlite::Result<(Vec<String>, usize)> {
    let Some(default_role) = manifest.default_role() else {
        return Ok((Vec::new(), 0));
    };
    let held: Vec<String> = conn
        .prepare("SELECT DISTINCT app_role FROM app_access WHERE app_id = ?1 ORDER BY app_role")?
        .query_map([app_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let gone: Vec<String> = held
        .into_iter()
        .filter(|role| !manifest.has_role(role))
        .collect();
    let mut users = 0;
    for role in &gone {
        users += changing(
            conn,
            &format!(
                "UPDATE app_access SET app_role = ?3 WHERE app_id = ?1 AND app_role = ?2 \
                 {CHANGED}"
            ),
            (app_id, role, &default_role.id),
        )?;
    }
    Ok((gone, users))
}

/// A user with access to an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub profile: Profile,
    pub active: bool,
    pub app_role: String,
}

const MEMBERS: &str = "SELECT u.id, u.username, u.email, u.display_name, u.role, u.status, \
                              x.app_role \
                       FROM app_access x JOIN users u ON u.id = x.user_id";

fn member(row: &rusqlite::Row<'_>) -> rusqlite::Result<Member> {
    let (profile, active) = users::profile(row)?;
    Ok(Member {
        profile,
        active,
        app_role: row.get(6)?,
    })
}

/// Users with access to the app, ordered by ID, after `after`, at most
/// `limit`.
pub fn members(
    conn: &Connection,
    app_id: &str,
    after: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<Member>> {
    conn.prepare(&format!(
        "{MEMBERS} WHERE x.app_id = ?1 AND u.id > ?2 ORDER BY u.id LIMIT ?3"
    ))?
    .query_map(
        (
            app_id,
            after.unwrap_or(""),
            i64::try_from(limit).unwrap_or(i64::MAX),
        ),
        member,
    )?
    .collect()
}

/// One user with access to the app.
pub fn member_of(
    conn: &Connection,
    app_id: &str,
    user_id: &str,
) -> rusqlite::Result<Option<Member>> {
    conn.query_row(
        &format!("{MEMBERS} WHERE x.app_id = ?1 AND u.id = ?2"),
        (app_id, user_id),
        member,
    )
    .optional()
}

/// The user's roles by app ID (the gateway's session cache).
pub fn roles(conn: &Connection, user_id: &str) -> rusqlite::Result<Vec<(String, String)>> {
    conn.prepare("SELECT app_id, app_role FROM app_access WHERE user_id = ?1")?
        .query_map([user_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

/// The user's role in the app, `None` without access.
pub fn role_of(conn: &Connection, user_id: &str, app_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT app_role FROM app_access WHERE user_id = ?1 AND app_id = ?2",
        (user_id, app_id),
        |row| row.get(0),
    )
    .optional()
}

/// What [`set`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The access was already as asked.
    Unchanged,
    /// Changed; the role before (`None`: no access before).
    Changed { previous: Option<String> },
}

/// Sets one user's access to one app: with `role` access with that role,
/// without none. Gives the app `access.changed` when anything changed. The
/// caller checks that the app is paired and has the role.
pub fn set(
    conn: &Connection,
    user_id: &str,
    app_id: &str,
    role: Option<&str>,
) -> rusqlite::Result<Outcome> {
    db::atomic(conn, |conn| {
        let previous = role_of(conn, user_id, app_id)?;
        if previous.as_deref() == role {
            return Ok(Outcome::Unchanged);
        }
        match role {
            Some(role) => conn.execute(
                "INSERT INTO app_access (user_id, app_id, app_role) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (user_id, app_id) DO UPDATE SET app_role = excluded.app_role",
                (user_id, app_id, role),
            )?,
            None => conn.execute(
                "DELETE FROM app_access WHERE user_id = ?1 AND app_id = ?2",
                (user_id, app_id),
            )?,
        };
        events::access_changed(conn, app_id, user_id, role)?;
        Ok(Outcome::Changed { previous })
    })
}

/// Every user's roles in the paired apps: user ID → slug → role.
pub fn all_by_user(
    conn: &Connection,
) -> rusqlite::Result<HashMap<String, BTreeMap<String, String>>> {
    let mut all: HashMap<String, BTreeMap<String, String>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT x.user_id, a.slug, x.app_role FROM app_access x \
         JOIN apps a ON a.id = x.app_id WHERE a.state = 'paired'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (user, slug, role) = row?;
        all.entry(user).or_default().insert(slug, role);
    }
    Ok(all)
}

/// One user's roles in the paired apps: slug → role.
pub fn of_user(conn: &Connection, user_id: &str) -> rusqlite::Result<BTreeMap<String, String>> {
    conn.prepare(
        "SELECT a.slug, x.app_role FROM app_access x \
         JOIN apps a ON a.id = x.app_id WHERE a.state = 'paired' AND x.user_id = ?1",
    )?
    .query_map([user_id], |row| Ok((row.get(0)?, row.get(1)?)))?
    .collect()
}

/// The number of users with access, by app ID.
pub fn counts(conn: &Connection) -> rusqlite::Result<HashMap<String, u64>> {
    conn.prepare("SELECT app_id, count(*) FROM app_access GROUP BY app_id")?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            ))
        })?
        .collect()
}

/// A user and their role in one app (`None`: no access).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserAccess {
    pub profile: Profile,
    pub active: bool,
    pub app_role: Option<String>,
}

/// Every user (active and locked) with their role in the app, ordered by
/// name.
pub fn users_of(conn: &Connection, app_id: &str) -> rusqlite::Result<Vec<UserAccess>> {
    conn.prepare(
        "SELECT u.id, u.username, u.email, u.display_name, u.role, u.status, x.app_role \
         FROM users u LEFT JOIN app_access x ON x.user_id = u.id AND x.app_id = ?1 \
         ORDER BY u.display_name COLLATE NOCASE, u.id",
    )?
    .query_map([app_id], |row| {
        let (profile, active) = users::profile(row)?;
        Ok(UserAccess {
            profile,
            active,
            app_role: row.get(6)?,
        })
    })?
    .collect()
}

/// What an invitation chose per app ID: a role or `None` for "no access".
/// Apps without an entry get their default.
pub type Choices = HashMap<String, Option<String>>;

/// The choices of one invitation.
pub fn choices(conn: &Connection, invitation_id: &str) -> rusqlite::Result<Choices> {
    conn.prepare("SELECT app_id, app_role FROM invitation_access WHERE invitation_id = ?1")?
        .query_map([invitation_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

/// The choices of every invitation, by invitation ID.
pub fn all_choices(conn: &Connection) -> rusqlite::Result<HashMap<String, Choices>> {
    let mut all: HashMap<String, Choices> = HashMap::new();
    let mut stmt = conn.prepare("SELECT invitation_id, app_id, app_role FROM invitation_access")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (invitation, app, role) = row?;
        all.entry(invitation).or_default().insert(app, role);
    }
    Ok(all)
}

/// Stores the invitation's choice for one app; returns the access the
/// invitation gave before ([`effective`] for that app) if it changes.
pub fn set_choice(
    conn: &Connection,
    invitation_id: &str,
    app: &App,
    role: Option<&str>,
) -> rusqlite::Result<Outcome> {
    db::atomic(conn, |conn| {
        let before = choices(conn, invitation_id)?;
        let previous = effective_for(app, before.get(&app.id));
        conn.execute(
            "INSERT INTO invitation_access (invitation_id, app_id, app_role) VALUES (?1, ?2, ?3) \
             ON CONFLICT (invitation_id, app_id) DO UPDATE SET app_role = excluded.app_role",
            (invitation_id, &app.id, role),
        )?;
        Ok(if previous.as_deref() == role {
            Outcome::Unchanged
        } else {
            Outcome::Changed { previous }
        })
    })
}

/// The role the choice gives in a paired app: the chosen role while the
/// manifest still has it (else its default role), none for "no access",
/// and without a choice the app's default ("everyone": the default role).
pub fn effective_for(app: &App, choice: Option<&Option<String>>) -> Option<String> {
    let manifest = app
        .manifest
        .as_ref()
        .filter(|_| app.status == Status::Paired)?;
    let default = || manifest.default_role().map(|role| role.id.clone());
    match choice {
        Some(Some(role)) if manifest.has_role(role) => Some(role.clone()),
        Some(Some(_)) => default(),
        Some(None) => None,
        None if app.access_default == AccessDefault::All => default(),
        None => None,
    }
}

/// What the choices give in the paired apps of `apps`: slug → role.
pub fn effective(apps: &[App], choices: Option<&Choices>) -> BTreeMap<String, String> {
    apps.iter()
        .filter_map(|app| {
            let choice = choices.and_then(|choices| choices.get(&app.id));
            effective_for(app, choice).map(|role| (app.slug.clone(), role))
        })
        .collect()
}

/// Access for a user who accepted an invitation: what the invitation chose,
/// the default for the other apps (instead of [`grant_to_new_user`]).
/// Returns slug → role of the access granted.
pub fn grant_invited(
    conn: &Connection,
    user_id: &str,
    invitation_id: &str,
) -> rusqlite::Result<BTreeMap<String, String>> {
    db::atomic(conn, |conn| {
        let chosen = choices(conn, invitation_id)?;
        let mut granted = BTreeMap::new();
        let mut paired: Vec<App> = apps::list(conn)?
            .into_iter()
            .filter(|app| app.status == Status::Paired)
            .collect();
        // Events in the order of `grant_to_new_user`: by app ID.
        paired.sort_by(|a, b| a.id.cmp(&b.id));
        for app in &paired {
            if let Some(role) = effective_for(app, chosen.get(&app.id)) {
                set(conn, user_id, &app.id, Some(&role))?;
                granted.insert(app.slug.clone(), role);
            }
        }
        Ok(granted)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::db::repo::{
        apps::{self, tests::manifest},
        users::{
            Role,
            tests::{add, conn},
        },
    };

    #[test]
    fn defaults_after_pairing_and_for_new_users() {
        let conn = conn();
        let anna = add(&conn, "anna", "anna@example.com", Role::Admin);
        let max = add(&conn, "max", "max@example.com", Role::User);
        let mail = apps::insert(&conn, "mail", "http://mail.test").unwrap();
        let notes = apps::insert(&conn, "notes", "http://notes.test").unwrap();
        conn.execute(
            "UPDATE apps SET access_default = 'selected' WHERE id = ?1",
            [&notes],
        )
        .unwrap();
        // Pending apps give no access to new users.
        let early = add(&conn, "early", "early@example.com", Role::User);
        assert_eq!(role_of(&conn, &early, &mail).unwrap(), None);

        apps::pair(&conn, &mail, &manifest(), &[1; 32]).unwrap();
        assert_eq!(grant_after_pairing(&conn, &mail, &manifest()).unwrap(), 3);
        assert_eq!(
            role_of(&conn, &anna, &mail).unwrap().as_deref(),
            Some("admin")
        );
        assert_eq!(
            role_of(&conn, &max, &mail).unwrap().as_deref(),
            Some("user")
        );
        apps::pair(&conn, &notes, &manifest(), &[2; 32]).unwrap();
        assert_eq!(grant_after_pairing(&conn, &notes, &manifest()).unwrap(), 1);
        assert_eq!(role_of(&conn, &max, &notes).unwrap(), None, "selected only");
        assert_eq!(
            role_of(&conn, &anna, &notes).unwrap().as_deref(),
            Some("admin")
        );

        // Pairing again keeps what was changed.
        conn.execute(
            "UPDATE app_access SET app_role = 'user' WHERE user_id = ?1",
            [&anna],
        )
        .unwrap();
        assert_eq!(grant_after_pairing(&conn, &mail, &manifest()).unwrap(), 0);
        assert_eq!(
            role_of(&conn, &anna, &mail).unwrap().as_deref(),
            Some("user")
        );

        // A new user gets the default role of "everyone" apps only (the
        // `users::insert` path).
        let lena = add(&conn, "lena", "lena@example.com", Role::Admin);
        assert_eq!(
            role_of(&conn, &lena, &mail).unwrap().as_deref(),
            Some("user")
        );
        assert_eq!(role_of(&conn, &lena, &notes).unwrap(), None);
        assert_eq!(grant_to_new_user(&conn, &lena).unwrap(), 0, "already there");

        // Members in `sub` order, paged.
        let all = members(&conn, &mail, None, 10).unwrap();
        let mut ids = vec![anna.clone(), max.clone(), early.clone(), lena.clone()];
        ids.sort();
        assert_eq!(
            all.iter().map(|m| m.profile.id.clone()).collect::<Vec<_>>(),
            ids
        );
        let page = members(&conn, &mail, Some(&ids[1]), 1).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].profile.id, ids[2]);
        assert!(member_of(&conn, &notes, &max).unwrap().is_none());
        let member = member_of(&conn, &mail, &max).unwrap().unwrap();
        assert_eq!((member.app_role.as_str(), member.active), ("user", true));

        // Removing a user or an app removes the entries.
        users::delete(&conn, &max).unwrap();
        assert!(member_of(&conn, &mail, &max).unwrap().is_none());
        apps::delete(&conn, &mail).unwrap();
        assert!(members(&conn, &mail, None, 10).unwrap().is_empty());
    }

    #[test]
    fn removed_roles_fall_back_to_the_default() {
        let conn = conn();
        let anna = add(&conn, "anna", "anna@example.com", Role::Admin);
        let max = add(&conn, "max", "max@example.com", Role::User);
        let mail = apps::insert(&conn, "mail", "http://mail.test").unwrap();
        apps::pair(&conn, &mail, &manifest(), &[1; 32]).unwrap();
        grant_after_pairing(&conn, &mail, &manifest()).unwrap();
        assert_eq!(
            fall_back_to_default(&conn, &mail, &manifest()).unwrap(),
            (vec![], 0)
        );
        // The admin role is renamed; `user` becomes the new default `reader`.
        let mut value = serde_json::to_value(manifest()).unwrap();
        value["roles"][1]["id"] = json!("owner");
        value["roles"][0]["id"] = json!("reader");
        let next = Manifest::parse(&value).unwrap();
        assert_eq!(
            fall_back_to_default(&conn, &mail, &next).unwrap(),
            (vec!["admin".to_owned(), "user".to_owned()], 2)
        );
        for user in [&anna, &max] {
            assert_eq!(
                role_of(&conn, user, &mail).unwrap().as_deref(),
                Some("reader")
            );
        }
    }
}
