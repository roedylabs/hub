//! Data access per table. The functions are synchronous and run in
//! `db::call` or in `spawn_blocking`.

pub mod app_settings;
pub mod users;
