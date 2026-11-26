//! Roedy Hub – server. The `roedy-hub` binary (`main.rs`) only calls
//! [`cli::run`]; the modules live here so that tests can use them.

pub mod api;
pub mod app;
pub mod auth;
pub mod cli;
pub mod client_ip;
pub mod config;
pub mod crypto;
pub mod db;
pub mod healthcheck;
pub mod i18n;
pub mod logging;
pub mod random;
pub mod rate_limit;
pub mod settings;
pub mod setup_token;
pub mod setup_totp;
pub mod web;
