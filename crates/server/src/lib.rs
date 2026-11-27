//! Roedy Hub – server. The `roedy-hub` binary (`main.rs`) only calls
//! [`cli::run`]; the modules live here so that tests can use them.

pub mod api;
pub mod app;
pub mod apps;
pub mod auth;
pub mod cli;
pub mod client_ip;
pub mod config;
pub mod crypto;
pub mod db;
pub mod events;
pub mod gateway;
pub mod healthcheck;
pub mod i18n;
pub mod identity;
pub mod instance;
pub mod internal;
pub mod jobs;
pub mod locks;
pub mod logging;
pub mod logo;
pub mod mailer;
pub mod notices;
pub mod random;
pub mod rate_limit;
pub mod reset_admin;
pub mod settings;
pub mod setup_token;
pub mod setup_totp;
pub mod signing;
pub mod web;
