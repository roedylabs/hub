//! Minimal JMAP client (RFC 8620, RFC 8621) for the hub's system mails:
//! session, requests with several method calls and sending
//! (`Identity/get`, `Mailbox/get`, `Email/set` and `EmailSubmission/set` with
//! references to created objects).
//!
//! The caller passes the HTTP client; it follows no redirects. Credentials
//! only go to the server's origin: redirects of the session and the
//! `apiUrl` must stay there. Responses are read up to 256 KiB at most.
//! Errors ([`Error`]) and `Debug` never contain passwords or tokens.
//!
//! Copied from Roedy Mail (`crates/jmap`) without the storage query
//! (`Quota/get`) and the fuzzing entry points.

mod client;
mod error;
mod mail;
mod request;
mod session;

pub use client::{Auth, Client, read_json, same_origin};
pub use error::Error;
pub use mail::{Identity, Message, Outbox, Sent};
pub use request::{CallId, Request, Response};
pub use session::Session;

/// Capabilities (RFC 8620, RFC 8621) the client uses.
pub const CORE: &str = "urn:ietf:params:jmap:core";
pub const MAIL: &str = "urn:ietf:params:jmap:mail";
pub const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
