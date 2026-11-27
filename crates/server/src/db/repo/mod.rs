//! Data access per table. The functions are synchronous and run in
//! `db::call` or in `spawn_blocking`.

pub mod app_access;
pub mod app_settings;
pub mod apps;
pub mod assets;
pub mod audit_log;
pub mod deleted_users;
pub mod email_changes;
pub mod events;
pub mod invitations;
pub mod known_devices;
pub mod password_resets;
pub mod recovery_codes;
pub mod sessions;
pub mod signing_keys;
pub mod totp_devices;
pub mod user_settings;
pub mod users;

use rusqlite::Connection;

/// Discards all open links from a user's mails: email address changes (also
/// reserved ones) and password reset links. For the steps `security_email`
/// recommends (change or reset the password, "Sign out all others") and after
/// confirming a new address – a link sent to the previous one must have no
/// effect after that. Taken over from Roedy Mail (`db::repo::discard_links`).
pub fn discard_links(conn: &Connection, user_id: &str) -> rusqlite::Result<()> {
    email_changes::discard(conn, user_id)?;
    password_resets::discard(conn, user_id)?;
    Ok(())
}
