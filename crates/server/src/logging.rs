//! Logging with `tracing`. Logs go to stderr so that stdout stays
//! free for the output of subcommands; `docker logs` shows both.
//!
//! Taken over from Roedy Mail (`crates/server/src/logging.rs`) without the
//! classifier for expected failures (maintenance mode); the token paths are
//! those of the hub's pages.

use std::{borrow::Cow, io::IsTerminal as _};

use anyhow::{Context as _, anyhow};
use axum::http::Request;
use tracing::Span;
use tracing_subscriber::{EnvFilter, filter::Directive};

use crate::config::{Config, LogFormat};

/// Target of the setup token message. It appears regardless of `ROEDY_LOG`,
/// because the initial setup is impossible without it.
pub const SETUP_TARGET: &str = "setup";

pub fn init(config: &Config) -> anyhow::Result<()> {
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter(&config.log)?)
        .with_writer(std::io::stderr);
    match config.log_format {
        LogFormat::Json => builder.json().flatten_event(true).try_init(),
        LogFormat::Text => builder
            .with_ansi(std::io::stderr().is_terminal())
            .try_init(),
    }
    .map_err(|e| anyhow!(e).context("Logging could not be set up"))
}

/// Paths whose next segment is a token from a mail link (tokens never
/// appear in logs). The SPA routes are included because the server serves
/// them itself: whoever opens the link from the mail loads the page directly
/// from here. The API paths are those Roedy Mail uses for the same links; for
/// `/api/v1/invites/` this also hits IDs (`invites/:id/resend`), which the log
/// does not miss.
const TOKEN_PREFIXES: &[&str] = &[
    "/api/v1/auth/password/reset/",
    "/api/v1/invites/",
    "/invite/",
    "/login/reset/",
    "/account/confirm-email/",
];

/// Replacement for a redacted segment.
const REDACTED: &str = "***";

/// Path for the request log with token segments redacted. The query is left
/// out entirely (e.g. `next` in `/login?next=…`).
pub fn redact_path(path: &str) -> Cow<'_, str> {
    for prefix in TOKEN_PREFIXES {
        if let Some(rest) = path.strip_prefix(prefix) {
            if rest.is_empty() {
                break;
            }
            let tail = rest.find('/').map_or("", |i| &rest[i..]);
            return Cow::Owned(format!("{prefix}{REDACTED}{tail}"));
        }
    }
    Cow::Borrowed(path)
}

/// Span per request for `TraceLayer` (level `debug`) – only method and
/// [`redact_path`] instead of the full URI. Same target as `DefaultMakeSpan`,
/// so that `ROEDY_LOG=…,tower_http=debug` enables span and events together.
pub fn request_span<B>(request: &Request<B>) -> Span {
    tracing::debug_span!(
        target: "tower_http::trace::make_span",
        "request",
        method = %request.method(),
        path = %redact_path(request.uri().path()),
    )
}

fn filter(log: &str) -> anyhow::Result<EnvFilter> {
    let setup: Directive = format!("{SETUP_TARGET}=info").parse()?;
    Ok(EnvFilter::try_new(log)
        .with_context(|| format!("ROEDY_LOG is invalid: \"{log}\""))?
        .add_directive(setup))
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    use super::*;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Returns what arrives of two info messages with `ROEDY_LOG=log`.
    fn logged_with(log: &str) -> String {
        let buffer = Buffer::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter(log).unwrap())
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: SETUP_TARGET, "Setup-Token: TEST");
            tracing::info!(target: "other", "other message");
        });
        String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn setup_token_survives_restrictive_filters() {
        for log in ["warn", "off", "roedy_hub_server=debug"] {
            let out = logged_with(log);
            assert!(out.contains("Setup-Token: TEST"), "{log}: {out}");
            assert!(!out.contains("other message"), "{log}: {out}");
        }
        assert!(logged_with("info").contains("other message"));
    }

    #[test]
    fn redacts_token_segments() {
        for (path, logged) in [
            ("/invite/K7Q2-abc", "/invite/***"),
            ("/login/reset/abc/", "/login/reset/***/"),
            (
                "/account/confirm-email/abc.def",
                "/account/confirm-email/***",
            ),
            (
                "/api/v1/auth/password/reset/abc",
                "/api/v1/auth/password/reset/***",
            ),
            ("/api/v1/invites/abc/accept", "/api/v1/invites/***/accept"),
            ("/api/v1/invites", "/api/v1/invites"),
            ("/api/v1/invites/", "/api/v1/invites/"),
            ("/invite", "/invite"),
            ("/login/reset", "/login/reset"),
            ("/login", "/login"),
            ("/account/profile", "/account/profile"),
        ] {
            assert_eq!(redact_path(path), logged, "{path}");
        }
    }

    #[test]
    fn request_log_contains_no_token_or_query() {
        // Also with `tower_http=debug` alone (same target as DefaultMakeSpan).
        for log in ["debug", "info,tower_http=debug"] {
            let buffer = Buffer::default();
            let writer = buffer.clone();
            let subscriber = tracing_subscriber::fmt()
                .with_env_filter(filter(log).unwrap())
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .finish();
            tracing::subscriber::with_default(subscriber, || {
                for uri in [
                    "/invite/SECRET1",
                    "/login/reset/SECRET2?next=SECRET3",
                    "/setup?token=SECRET4",
                ] {
                    let request = Request::get(uri).body(()).unwrap();
                    request_span(&request)
                        .in_scope(|| tracing::debug!(target: "tower_http::trace", "request"));
                }
            });
            let out = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
            assert!(out.contains("path=/invite/***"), "{log}: {out}");
            assert!(out.contains("path=/login/reset/***"), "{log}: {out}");
            assert!(out.contains("path=/setup"), "{log}: {out}");
            assert!(!out.contains("SECRET"), "{log}: {out}");
        }
    }

    #[test]
    fn rejects_invalid_filter() {
        let err = filter("[[").unwrap_err();
        assert!(err.to_string().contains("ROEDY_LOG"), "{err}");
    }
}
