//! Data access per table. The functions are synchronous and run in
//! `db::call` or in `spawn_blocking`.

pub mod app_settings;
pub mod audit_log;
pub mod recovery_codes;
pub mod sessions;
pub mod totp_devices;
pub mod user_settings;
pub mod users;
