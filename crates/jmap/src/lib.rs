//! Minimal JMAP client (RFC 8620, RFC 8621) for the hub's system mails:
//! session, requests with several method calls and sending
//! (`Identity/get`, `Mailbox/get`, `Email/set` and `EmailSubmission/set` with
//! references to created objects) and the storage of an account
//! (`Quota/get`, RFC 9425) for the check of users' Stalwart accounts.
//!
//! The caller passes the HTTP client, which must not follow redirects itself
//! (`.redirect(reqwest::redirect::Policy::none())`): the crate follows them,
//! only on the server's origin, so that credentials only go there –
//! redirects of the session and the `apiUrl` must stay on it. Responses are read up to 256 KiB at most.
//! Errors ([`Error`]) and `Debug` never contain passwords or tokens.
//!
//! Copied from Roedy Mail (`crates/jmap`) without the fuzzing entry points.

mod client;
mod error;
mod mail;
mod quota;
mod request;
mod session;

pub use client::{Auth, Client, read_json, same_origin};
pub use error::Error;
pub use mail::{Identity, Message, Outbox, Sent};
pub use quota::{QUOTA, Storage};
pub use request::{CallId, Request, Response};
pub use session::Session;

/// Capabilities (RFC 8620, RFC 8621) the client uses.
pub const CORE: &str = "urn:ietf:params:jmap:core";
pub const MAIL: &str = "urn:ietf:params:jmap:mail";
pub const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
