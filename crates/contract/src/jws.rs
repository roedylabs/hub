//! Compact JWS (RFC 7515) with Ed25519, exactly as the contract uses it
//! (§ 4.2, § 4.3, § 10.1): header `{"alg":"EdDSA","typ":…,"kid":…}`,
//! segments in base64url without padding.
//!
//! Verification follows contract § 4.3 step by step and stops at the first
//! failure:
//!
//! 1. at most [`MAX_TOKEN_LEN`] bytes, three segments, canonical base64url,
//!    header a JSON object without a member name twice (at any depth, as
//!    for the claims), no `crit` member whatever its value;
//! 2. `alg` = `EdDSA`, `typ` = the expected kind, a `kid` – `alg` is checked
//!    before anything else, so `none` or `HS256` never get further;
//! 3. `kid` in the JWKS ([`Error::UnknownKey`]: refresh the JWKS and retry);
//! 4. the Ed25519 signature over the first two segments as sent;
//! 5. only then the claims: a JSON object with `iss`, `aud` (a string),
//!    `iat` and `exp`; issuer, audience, expiry, `iat` and lifetime.

use std::{collections::HashSet, fmt};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair as _, UnparsedPublicKey};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor},
};

use crate::{ALG, Claims, Jwk, Jwks, LEEWAY, MAX_TOKEN_LEN};

/// Why a token was rejected. For an app every error means "not signed in"
/// (contract § 4.3); [`Error::code`] is the name in the test vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Error {
    /// Not a compact JWS of the contract: too long, not three segments, not
    /// base64url without padding, header not a JSON object, without `kid` or
    /// with `crit`.
    Malformed,
    /// `alg` is not `EdDSA` (including `none` and a missing `alg`).
    Algorithm,
    /// `typ` is not the expected one, e.g. an event where an identity was
    /// expected.
    Type,
    /// No usable key with this `kid` in the JWKS; refresh it and try again.
    UnknownKey,
    /// The signature does not match the header and claims.
    Signature,
    /// Signed, but the claims are not a JSON object of the expected shape
    /// (e.g. `aud` as an array, a claim missing or of the wrong type).
    Claims,
    /// `iss` is not the expected issuer.
    Issuer,
    /// `aud` is not the expected app.
    Audience,
    /// `exp` ≤ now − [`LEEWAY`].
    Expired,
    /// `iat` > now + [`LEEWAY`].
    IssuedInFuture,
    /// `exp − iat` is negative or longer than the kind allows.
    Lifetime,
}

impl Error {
    /// Stable name, as in `docs/app-contract/vectors.json`.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::Algorithm => "alg",
            Self::Type => "typ",
            Self::UnknownKey => "unknown_kid",
            Self::Signature => "signature",
            Self::Claims => "claims",
            Self::Issuer => "iss",
            Self::Audience => "aud",
            Self::Lifetime => "lifetime",
            Self::Expired => "expired",
            Self::IssuedInFuture => "iat",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "not a compact JWS of the Roedy contract",
            Self::Algorithm => "algorithm is not EdDSA",
            Self::Type => "unexpected token type",
            Self::UnknownKey => "unknown key ID",
            Self::Signature => "invalid signature",
            Self::Claims => "claims have the wrong shape",
            Self::Issuer => "wrong issuer",
            Self::Audience => "wrong audience",
            Self::Lifetime => "lifetime too long",
            Self::Expired => "expired",
            Self::IssuedInFuture => "issued in the future",
        })
    }
}

impl std::error::Error for Error {}

/// What the verifier expects besides the signature.
#[derive(Debug, Clone, Copy)]
pub struct Expected<'a> {
    /// The issuer stored at pairing: the hub's public URL without trailing
    /// slash.
    pub issuer: &'a str,
    /// The app's own slug.
    pub audience: &'a str,
    /// Current time, Unix seconds.
    pub now: i64,
}

/// Verifies `token` as claims of kind `C` (its `typ`) against the keys in
/// `keys`; see the module documentation for the steps.
pub fn verify<C: Claims>(token: &str, keys: &Jwks, expected: &Expected<'_>) -> Result<C, Error> {
    if token.len() > MAX_TOKEN_LEN {
        return Err(Error::Malformed);
    }
    let mut segments = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return Err(Error::Malformed);
    };

    let signed = &token[..header.len() + 1 + payload.len()];
    let header: Header = object(&decode(header)?).ok_or(Error::Malformed)?;
    if header.alg.as_deref() != Some(ALG) {
        return Err(Error::Algorithm);
    }
    if header.typ.as_deref() != Some(C::TYP.as_str()) {
        return Err(Error::Type);
    }
    if header.crit {
        return Err(Error::Malformed);
    }
    let kid = header.kid.ok_or(Error::Malformed)?;
    let public = keys.find(&kid).ok_or(Error::UnknownKey)?;

    let signature = decode(signature)?;
    let payload = decode(payload)?;
    UnparsedPublicKey::new(&ED25519, public)
        .verify(signed.as_bytes(), &signature)
        .map_err(|_| Error::Signature)?;

    let registered: Registered = object(&payload).ok_or(Error::Claims)?;
    if registered.iss != expected.issuer {
        return Err(Error::Issuer);
    }
    if registered.aud != expected.audience {
        return Err(Error::Audience);
    }
    if registered.exp <= expected.now.saturating_sub(LEEWAY) {
        return Err(Error::Expired);
    }
    if registered.iat > expected.now.saturating_add(LEEWAY) {
        return Err(Error::IssuedInFuture);
    }
    match registered.exp.checked_sub(registered.iat) {
        Some(lifetime) if (0..=C::TYP.max_lifetime()).contains(&lifetime) => {}
        _ => return Err(Error::Lifetime),
    }
    object(&payload).ok_or(Error::Claims)
}

/// Base64url without padding; non-canonical encodings (trailing bits) are
/// errors, so that every token has exactly one spelling.
fn decode(segment: &str) -> Result<Vec<u8>, Error> {
    URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|_| Error::Malformed)
}

/// `T` from a JSON object. serde would also read a struct from an array by
/// position, and would take an unknown member twice without noticing; the
/// contract only has objects, each member name once.
fn object<T: de::DeserializeOwned>(json: &[u8]) -> Option<T> {
    let start = json
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))?;
    if json[start] != b'{' {
        return None;
    }
    serde_json::from_slice::<Unique>(json).ok()?;
    serde_json::from_slice(json).ok()
}

/// Any JSON value in which no object has a member name twice, at any depth.
struct Unique;

impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(Unique)
    }
}

impl<'de> Visitor<'de> for Unique {
    type Value = Unique;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> Result<Unique, E> {
        Ok(Unique)
    }

    fn visit_i64<E>(self, _: i64) -> Result<Unique, E> {
        Ok(Unique)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Unique, E> {
        Ok(Unique)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Unique, E> {
        Ok(Unique)
    }

    fn visit_str<E>(self, _: &str) -> Result<Unique, E> {
        Ok(Unique)
    }

    fn visit_unit<E>(self) -> Result<Unique, E> {
        Ok(Unique)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Unique, A::Error> {
        while seq.next_element::<Unique>()?.is_some() {}
        Ok(Unique)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Unique, A::Error> {
        let mut names = HashSet::new();
        while let Some(name) = map.next_key::<String>()? {
            if !names.insert(name) {
                return Err(de::Error::custom("duplicate member"));
            }
            map.next_value::<Unique>()?;
        }
        Ok(Unique)
    }
}

/// `true` for a present member, whatever its value (also `null`).
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    IgnoredAny::deserialize(deserializer).map(|_| true)
}

/// JOSE header as read. Other members are ignored, except `crit`: a
/// verifier must reject extensions it does not understand (RFC 7515
/// § 4.1.11), and the contract has none.
#[derive(Deserialize)]
struct Header {
    alg: Option<String>,
    typ: Option<String>,
    kid: Option<String>,
    #[serde(default, deserialize_with = "present")]
    crit: bool,
}

/// JOSE header as written, in the contract's order.
#[derive(Serialize)]
struct SignedHeader<'a> {
    alg: &'a str,
    typ: &'a str,
    kid: &'a str,
}

/// The claims every kind has, checked before the kind's own.
#[derive(Deserialize)]
struct Registered {
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
}

/// An Ed25519 private key with its key ID. `Debug` does not show the key.
pub struct SigningKey {
    kid: String,
    pair: Ed25519KeyPair,
}

/// The seed is not a usable Ed25519 key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidKey;

impl fmt::Display for InvalidKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid Ed25519 key")
    }
}

impl std::error::Error for InvalidKey {}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningKey")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

impl SigningKey {
    /// The key for a 32-byte seed (the private key of RFC 8032, `d` of an
    /// OKP JWK). The caller makes the seed from a secure random source and
    /// keeps it secret.
    pub fn from_seed(kid: impl Into<String>, seed: &[u8; 32]) -> Result<Self, InvalidKey> {
        Ok(Self {
            kid: kid.into(),
            pair: Ed25519KeyPair::from_seed_unchecked(seed).map_err(|_| InvalidKey)?,
        })
    }

    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The public key.
    pub fn public_key(&self) -> [u8; 32] {
        let mut public = [0; 32];
        public.copy_from_slice(self.pair.public_key().as_ref());
        public
    }

    /// The public key as it is published.
    pub fn jwk(&self) -> Jwk {
        Jwk::ed25519(&self.kid, &self.public_key())
    }

    /// Signs `claims` with the header `{"alg":"EdDSA","typ":…,"kid":…}`.
    /// Ed25519 is deterministic: the same claims give the same token.
    pub fn sign<C: Claims>(&self, claims: &C) -> Result<String, serde_json::Error> {
        let header = serde_json::to_vec(&SignedHeader {
            alg: ALG,
            typ: C::TYP.as_str(),
            kid: &self.kid,
        })?;
        Ok(self.sign_raw(&header, &serde_json::to_vec(claims)?))
    }

    /// Signs `header` and `payload` exactly as given, without any check. For
    /// tests and test vectors that need tokens the hub never makes; use
    /// [`Self::sign`] otherwise.
    pub fn sign_raw(&self, header: &[u8], payload: &[u8]) -> String {
        let mut token = String::with_capacity((header.len() + payload.len() + 64) * 4 / 3 + 3);
        URL_SAFE_NO_PAD.encode_string(header, &mut token);
        token.push('.');
        URL_SAFE_NO_PAD.encode_string(payload, &mut token);
        let signature = self.pair.sign(token.as_bytes());
        token.push('.');
        URL_SAFE_NO_PAD.encode_string(signature.as_ref(), &mut token);
        token
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::{Event, Identity, Typ};

    const NOW: i64 = 1_795_600_150;
    const ISSUER: &str = "https://app.example.com";

    fn key() -> SigningKey {
        SigningKey::from_seed("k1", &[1; 32]).unwrap()
    }

    fn jwks() -> Jwks {
        Jwks {
            keys: vec![key().jwk()],
        }
    }

    fn expected() -> Expected<'static> {
        Expected {
            issuer: ISSUER,
            audience: "mail",
            now: NOW,
        }
    }

    fn identity() -> Identity {
        Identity {
            iss: ISSUER.into(),
            aud: "mail".into(),
            sub: "01JDX2Q7B5W3Y1Z9X7V5T3R1P0".into(),
            username: "alex".into(),
            email: "alex@example.com".into(),
            name: "Alex Example".into(),
            role: "user".into(),
            app_role: "user".into(),
            locale: "de".into(),
            sid: "01JDX3A1C3E5G7J9K1M3N5P7Q9".into(),
            auth_time: NOW - 1000,
            iat: NOW - 20,
            exp: NOW + 40,
        }
    }

    fn event() -> Event {
        Event {
            iss: ISSUER.into(),
            aud: "mail".into(),
            jti: "01JDX5R2T4V6X8Z0B2D4F6H8J0".into(),
            seq: 42,
            kind: "user.locked".into(),
            data: json!({ "sub": "01JDX2Q7B5W3Y1Z9X7V5T3R1P0" }),
            iat: NOW - 100,
            exp: NOW + 200,
        }
    }

    fn b64(json: &Value) -> String {
        URL_SAFE_NO_PAD.encode(json.to_string())
    }

    fn header(typ: &str) -> Value {
        json!({ "alg": "EdDSA", "typ": typ, "kid": "k1" })
    }

    fn claims_json() -> Value {
        serde_json::to_value(identity()).unwrap()
    }

    /// A token with `header` and `claims` signed by [`key`].
    fn signed(header: &Value, claims: &Value) -> String {
        key().sign_raw(header.to_string().as_bytes(), claims.to_string().as_bytes())
    }

    fn with_claim(name: &str, value: Value) -> String {
        let mut claims = claims_json();
        claims[name] = value;
        signed(&header("roedy-id+jwt"), &claims)
    }

    fn with_header(change: impl FnOnce(&mut Value)) -> String {
        let mut header = header("roedy-id+jwt");
        change(&mut header);
        signed(&header, &claims_json())
    }

    fn verify_identity(token: &str) -> Result<Identity, Error> {
        verify::<Identity>(token, &jwks(), &expected())
    }

    #[test]
    fn signs_and_verifies_both_kinds() {
        let token = key().sign(&identity()).unwrap();
        assert_eq!(verify_identity(&token), Ok(identity()));
        // Deterministic: the same claims give the same token.
        assert_eq!(token, key().sign(&identity()).unwrap());

        let header = token.split('.').next().unwrap();
        assert_eq!(
            URL_SAFE_NO_PAD.decode(header).unwrap(),
            br#"{"alg":"EdDSA","typ":"roedy-id+jwt","kid":"k1"}"#
        );

        let token = key().sign(&event()).unwrap();
        assert_eq!(verify::<Event>(&token, &jwks(), &expected()), Ok(event()));
    }

    #[test]
    fn rejects_every_broken_token() {
        let valid = key().sign(&identity()).unwrap();
        let [h, p, s]: [&str; 3] = valid.split('.').collect::<Vec<_>>().try_into().unwrap();
        let other_key = SigningKey::from_seed("k1", &[2; 32]).unwrap();
        let unknown_kid = SigningKey::from_seed("k9", &[1; 32]).unwrap();
        let mut flipped = URL_SAFE_NO_PAD.decode(s).unwrap();
        flipped[0] ^= 1;
        let mut tampered = claims_json();
        tampered["sub"] = json!("01JDX2Q7B5W3Y1Z9X7V5T3R1P1");
        let event_token = key().sign(&event()).unwrap();
        // The last character of a 64-byte signature carries 4 unused bits;
        // setting one gives the same bytes in a second spelling.
        let mut non_canonical = valid.clone();
        let last = non_canonical.pop().unwrap();
        non_canonical.push(match last {
            'A' => 'B',
            'Q' => 'R',
            'g' => 'h',
            'w' => 'x',
            other => panic!("not canonical: {other}"),
        });

        let cases: Vec<(&str, String, Error)> = vec![
            ("empty", String::new(), Error::Malformed),
            ("two segments", format!("{h}.{p}"), Error::Malformed),
            ("four segments", format!("{valid}."), Error::Malformed),
            (
                "JWE (five segments)",
                format!("{h}.{p}.{s}.{p}.{s}"),
                Error::Malformed,
            ),
            ("padding", format!("{h}=.{p}.{s}"), Error::Malformed),
            (
                "non-canonical base64",
                non_canonical.clone(),
                Error::Malformed,
            ),
            (
                "too long",
                format!("{h}.{}.{s}", "A".repeat(MAX_TOKEN_LEN)),
                Error::Malformed,
            ),
            (
                "header is an array",
                format!("{}.{p}.{s}", b64(&json!(["EdDSA", "roedy-id+jwt", "k1"]))),
                Error::Malformed,
            ),
            (
                "header is not JSON",
                format!("{}.{p}.{s}", URL_SAFE_NO_PAD.encode("alg")),
                Error::Malformed,
            ),
            (
                "duplicate alg",
                format!(
                    "{}.{p}.{s}",
                    URL_SAFE_NO_PAD
                        .encode(r#"{"alg":"none","alg":"EdDSA","typ":"roedy-id+jwt","kid":"k1"}"#)
                ),
                Error::Malformed,
            ),
            (
                "alg none, no signature",
                format!(
                    "{}.{p}.",
                    b64(&json!({ "alg": "none", "typ": "roedy-id+jwt", "kid": "k1" }))
                ),
                Error::Algorithm,
            ),
            (
                "alg none, signed",
                with_header(|h| h["alg"] = json!("none")),
                Error::Algorithm,
            ),
            (
                "alg HS256",
                with_header(|h| h["alg"] = json!("HS256")),
                Error::Algorithm,
            ),
            (
                "alg Ed25519",
                with_header(|h| h["alg"] = json!("Ed25519")),
                Error::Algorithm,
            ),
            (
                "alg lowercase",
                with_header(|h| h["alg"] = json!("eddsa")),
                Error::Algorithm,
            ),
            (
                "alg missing",
                with_header(|h| {
                    h.as_object_mut().unwrap().remove("alg");
                }),
                Error::Algorithm,
            ),
            (
                "alg null",
                with_header(|h| h["alg"] = Value::Null),
                Error::Algorithm,
            ),
            ("event as identity", event_token.clone(), Error::Type),
            (
                "typ JWT",
                with_header(|h| h["typ"] = json!("JWT")),
                Error::Type,
            ),
            (
                "typ uppercase",
                with_header(|h| h["typ"] = json!("ROEDY-ID+JWT")),
                Error::Type,
            ),
            (
                "typ missing",
                with_header(|h| {
                    h.as_object_mut().unwrap().remove("typ");
                }),
                Error::Type,
            ),
            (
                "kid missing",
                with_header(|h| {
                    h.as_object_mut().unwrap().remove("kid");
                }),
                Error::Malformed,
            ),
            (
                "kid not a string",
                with_header(|h| h["kid"] = json!(1)),
                Error::Malformed,
            ),
            (
                "crit",
                with_header(|h| h["crit"] = json!(["exp"])),
                Error::Malformed,
            ),
            (
                "crit null",
                with_header(|h| h["crit"] = Value::Null),
                Error::Malformed,
            ),
            (
                "crit empty",
                with_header(|h| h["crit"] = json!([])),
                Error::Malformed,
            ),
            (
                "unknown header member twice",
                key().sign_raw(
                    br#"{"alg":"EdDSA","typ":"roedy-id+jwt","kid":"k1","x-future":1,"x-future":2}"#,
                    claims_json().to_string().as_bytes(),
                ),
                Error::Malformed,
            ),
            (
                "header member twice, once escaped",
                key().sign_raw(
                    br#"{"alg":"EdDSA","typ":"roedy-id+jwt","kid":"k1","x":1,"\u0078":2}"#,
                    claims_json().to_string().as_bytes(),
                ),
                Error::Malformed,
            ),
            (
                "unknown claim twice",
                {
                    let claims = claims_json().to_string();
                    let claims =
                        format!(r#"{},"extra":1,"extra":2}}"#, &claims[..claims.len() - 1]);
                    key().sign_raw(
                        header("roedy-id+jwt").to_string().as_bytes(),
                        claims.as_bytes(),
                    )
                },
                Error::Claims,
            ),
            (
                "known claim twice",
                {
                    let claims = claims_json().to_string();
                    let claims = format!(r#"{},"sub":"other"}}"#, &claims[..claims.len() - 1]);
                    key().sign_raw(
                        header("roedy-id+jwt").to_string().as_bytes(),
                        claims.as_bytes(),
                    )
                },
                Error::Claims,
            ),
            (
                "unknown kid",
                unknown_kid.sign(&identity()).unwrap(),
                Error::UnknownKey,
            ),
            (
                "embedded jwk is ignored",
                {
                    let mut header = header("roedy-id+jwt");
                    header["jwk"] = serde_json::to_value(other_key.jwk()).unwrap();
                    other_key.sign_raw(
                        header.to_string().as_bytes(),
                        claims_json().to_string().as_bytes(),
                    )
                },
                Error::Signature,
            ),
            (
                "forged signature",
                format!("{h}.{p}.{}", URL_SAFE_NO_PAD.encode(&flipped)),
                Error::Signature,
            ),
            (
                "signed by another key",
                other_key.sign(&identity()).unwrap(),
                Error::Signature,
            ),
            (
                "claims changed",
                format!("{h}.{}.{s}", b64(&tampered)),
                Error::Signature,
            ),
            (
                "signature of another token",
                format!(
                    "{h}.{p}.{}",
                    key()
                        .sign(&tampered_identity())
                        .unwrap()
                        .rsplit('.')
                        .next()
                        .unwrap()
                ),
                Error::Signature,
            ),
            (
                "short signature",
                format!("{h}.{p}.{}", &s[..20]),
                Error::Signature,
            ),
            ("empty signature", format!("{h}.{p}."), Error::Signature),
            (
                "claims are an array",
                signed(&header("roedy-id+jwt"), &json!([ISSUER, "mail", NOW, NOW])),
                Error::Claims,
            ),
            (
                "aud as array",
                with_claim("aud", json!(["mail"])),
                Error::Claims,
            ),
            (
                "exp as string",
                with_claim("exp", json!((NOW + 40).to_string())),
                Error::Claims,
            ),
            (
                "exp as fraction",
                with_claim("exp", json!(1_795_600_190.5)),
                Error::Claims,
            ),
            (
                "sub missing",
                {
                    let mut claims = claims_json();
                    claims.as_object_mut().unwrap().remove("sub");
                    signed(&header("roedy-id+jwt"), &claims)
                },
                Error::Claims,
            ),
            (
                "wrong issuer",
                with_claim("iss", json!("https://other.example.com")),
                Error::Issuer,
            ),
            (
                "issuer with trailing slash",
                with_claim("iss", json!("https://app.example.com/")),
                Error::Issuer,
            ),
            (
                "wrong aud",
                with_claim("aud", json!("contacts")),
                Error::Audience,
            ),
            (
                "aud uppercase",
                with_claim("aud", json!("Mail")),
                Error::Audience,
            ),
            (
                "lifetime 61 s",
                with_claim("exp", json!(NOW - 20 + 61)),
                Error::Lifetime,
            ),
            (
                "exp before iat",
                {
                    let mut claims = claims_json();
                    claims["iat"] = json!(NOW + 5);
                    claims["exp"] = json!(NOW + 4);
                    signed(&header("roedy-id+jwt"), &claims)
                },
                Error::Lifetime,
            ),
        ];
        for (name, token, error) in cases {
            assert_eq!(verify_identity(&token), Err(error), "{name}");
        }
    }

    fn tampered_identity() -> Identity {
        Identity {
            sub: "01JDX2Q7B5W3Y1Z9X7V5T3R1P1".into(),
            ..identity()
        }
    }

    #[test]
    fn an_identity_never_passes_as_an_event() {
        let token = key().sign(&identity()).unwrap();
        assert_eq!(
            verify::<Event>(&token, &jwks(), &expected()),
            Err(Error::Type)
        );
    }

    #[test]
    fn checks_times_with_the_leeway() {
        let at = |iat: i64, exp: i64, now: i64| {
            let token = key()
                .sign(&Identity {
                    iat,
                    exp,
                    ..identity()
                })
                .unwrap();
            verify::<Identity>(&token, &jwks(), &Expected { now, ..expected() })
        };
        let cases = [
            ("fresh", NOW, NOW + 60, NOW, Ok(())),
            ("just valid after exp", NOW, NOW + 60, NOW + 69, Ok(())),
            ("expired", NOW, NOW + 60, NOW + 70, Err(Error::Expired)),
            (
                "long expired",
                NOW,
                NOW + 60,
                NOW + 7200,
                Err(Error::Expired),
            ),
            ("iat 10 s ahead", NOW + 10, NOW + 70, NOW, Ok(())),
            (
                "iat 11 s ahead",
                NOW + 11,
                NOW + 71,
                NOW,
                Err(Error::IssuedInFuture),
            ),
            ("lifetime 0", NOW, NOW, NOW, Ok(())),
            ("lifetime 60", NOW - 60, NOW, NOW, Ok(())),
            ("lifetime 61", NOW - 61, NOW, NOW, Err(Error::Lifetime)),
            ("huge times", i64::MIN, i64::MAX, NOW, Err(Error::Lifetime)),
            (
                "extreme now",
                NOW,
                NOW + 60,
                i64::MIN,
                Err(Error::IssuedInFuture),
            ),
        ];
        for (name, iat, exp, now, result) in cases {
            assert_eq!(at(iat, exp, now).map(|_| ()), result, "{name}");
        }
    }

    #[test]
    fn rejects_a_member_twice_at_any_depth() {
        let header = header("roedy-event+jwt").to_string();
        let claims = serde_json::to_string(&event()).unwrap();
        let sign = |claims: &str| key().sign_raw(header.as_bytes(), claims.as_bytes());
        let verify_event = |token: &str| verify::<Event>(token, &jwks(), &expected()).map(|_| ());
        assert_eq!(verify_event(&sign(&claims)), Ok(()));
        let nested = claims.replace(r#""data":{"#, r#""data":{"sub":"x","#);
        assert_ne!(nested, claims);
        assert_eq!(verify_event(&sign(&nested)), Err(Error::Claims));
        let in_array = claims.replace(r#""data":{"#, r#""data":{"list":[{"a":1,"a":2}],"#);
        assert_eq!(verify_event(&sign(&in_array)), Err(Error::Claims));
        let distinct = claims.replace(r#""data":{"#, r#""data":{"list":[{"a":1},{"a":2}],"#);
        assert_eq!(verify_event(&sign(&distinct)), Ok(()));
    }

    #[test]
    fn events_may_live_five_minutes() {
        let at = |iat: i64, exp: i64| {
            let token = key()
                .sign(&Event {
                    iat,
                    exp,
                    ..event()
                })
                .unwrap();
            verify::<Event>(&token, &jwks(), &expected()).map(|_| ())
        };
        assert_eq!(at(NOW - 100, NOW + 200), Ok(()));
        assert_eq!(at(NOW - 100, NOW + 201), Err(Error::Lifetime));
        assert_eq!(Typ::Event.max_lifetime(), 300);
    }

    #[test]
    fn ignores_unknown_claims_and_header_members() {
        let mut claims = claims_json();
        claims["future_claim"] = json!({ "nested": true });
        let mut header = header("roedy-id+jwt");
        header["x-future"] = json!(1);
        assert_eq!(verify_identity(&signed(&header, &claims)), Ok(identity()));
    }

    #[test]
    fn kinds_parse_exactly() {
        assert_eq!(Typ::parse("roedy-id+jwt"), Some(Typ::Identity));
        assert_eq!(Typ::parse("roedy-event+jwt"), Some(Typ::Event));
        assert_eq!(Typ::parse("Roedy-ID+JWT"), None);
        assert_eq!(Typ::parse("JWT"), None);
    }

    #[test]
    fn debug_hides_the_private_key() {
        assert_eq!(format!("{:?}", key()), r#"SigningKey { kid: "k1", .. }"#);
    }
}
