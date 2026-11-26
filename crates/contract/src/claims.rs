//! Claims of the identity (contract § 4.2) and of an event (§ 10.1). Fields
//! are in the contract's order, which is also the order they are signed in.
//! Unknown claims are ignored when reading (contract § 14).

use serde::{Deserialize, Serialize};

use crate::{Claims, Typ};

/// The signed-in user, as the hub sends it to an app in
/// [`X-Roedy-Identity`](crate::IDENTITY_HEADER).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// The hub's public URL without trailing slash.
    pub iss: String,
    /// The app's slug; always a string, never an array.
    pub aud: String,
    /// User ID (ULID), stable for the user's lifetime.
    pub sub: String,
    pub username: String,
    pub email: String,
    pub name: String,
    /// Hub role: `admin` or `user`.
    pub role: String,
    /// The user's role in this app, an `id` from the app's manifest.
    pub app_role: String,
    /// `en` or `de`.
    pub locale: String,
    /// ID of the hub session (ULID).
    pub sid: String,
    /// Last full sign-in in this session, Unix seconds.
    pub auth_time: i64,
    pub iat: i64,
    pub exp: i64,
}

impl Claims for Identity {
    const TYP: Typ = Typ::Identity;
}

/// One event, as the hub delivers it to an app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub iss: String,
    pub aud: String,
    /// Event ID (ULID).
    pub jti: String,
    /// Position in this app's event stream.
    pub seq: u64,
    /// Event type, e.g. `session.revoked` (contract § 10.2).
    #[serde(rename = "type")]
    pub kind: String,
    /// Depends on [`Self::kind`].
    pub data: serde_json::Value,
    pub iat: i64,
    pub exp: i64,
}

impl Claims for Event {
    const TYP: Typ = Typ::Event;
}
