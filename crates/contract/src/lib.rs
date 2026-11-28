//! The Roedy Hub app contract, version 1 (`docs/app-contract.md`), as shared
//! types: the identity the hub sends to apps ([`Identity`], § 4), the events
//! ([`Event`], § 10.1), the public signing keys ([`Jwks`], § 4.4) and the
//! strict JWS that carries them ([`SigningKey::sign`], [`verify`]).
//!
//! The JWS is a small implementation on top of `ring` (Ed25519) instead of a
//! JWT library, so that the rules stay exactly the contract's: `alg` must be
//! `EdDSA` whatever the key says, `typ` decides which claims a token can carry
//! (an event never passes as an identity and the other way round), and the
//! times are checked with the contract's leeway.
//!
//! The manifest ([`Manifest`], § 8) with its rules, slugs, pairing codes and
//! the bodies of the internal API ([`pairing`], § 7 and § 9) are shared the
//! same way, so that the hub and the example app agree on them.
//!
//! The test vectors in `docs/app-contract/vectors.json` (contract § 15) are
//! made and checked with this crate (`tests/vectors.rs`); apps in other
//! languages can check their verification against them.

mod claims;
mod jwks;
mod jws;
pub mod manifest;
pub mod pairing;

pub use claims::{Event, Identity};
pub use jwks::{Jwk, Jwks};
pub use jws::{Error, Expected, InvalidKey, SigningKey, verify};
pub use manifest::{Manifest, ManifestError, Stalwart};

/// The request header with the identity (contract § 4.1).
pub const IDENTITY_HEADER: &str = "X-Roedy-Identity";

/// The only signature algorithm (contract § 4.2): Ed25519.
pub const ALG: &str = "EdDSA";

/// Seconds a verifier tolerates between its clock and the hub's when it
/// checks `exp` and `iat` (contract § 4.3).
pub const LEEWAY: i64 = 10;

/// Longest token [`verify`] reads, in bytes. The hub's tokens are far
/// smaller; anything longer is rejected before decoding.
pub const MAX_TOKEN_LEN: usize = 8 * 1024;

/// Kind of a token, its JOSE `typ` (contract § 4.2 and § 10.1). It decides
/// which claims a token carries and how long it may live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Typ {
    /// `roedy-id+jwt`: the identity header.
    Identity,
    /// `roedy-event+jwt`: the body of an event delivery.
    Event,
}

impl Typ {
    /// The `typ` header value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "roedy-id+jwt",
            Self::Event => "roedy-event+jwt",
        }
    }

    /// Longest allowed `exp − iat` in seconds.
    pub const fn max_lifetime(self) -> i64 {
        match self {
            Self::Identity => 60,
            Self::Event => 300,
        }
    }

    /// The kind for a `typ` header value; exact match only.
    pub fn parse(value: &str) -> Option<Self> {
        [Self::Identity, Self::Event]
            .into_iter()
            .find(|typ| typ.as_str() == value)
    }
}

/// Claims of one [`Typ`]: signing writes the matching `typ`, verifying
/// accepts only that `typ`.
pub trait Claims: serde::Serialize + serde::de::DeserializeOwned {
    const TYP: Typ;
}
