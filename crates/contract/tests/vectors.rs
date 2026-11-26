//! Test vectors of the contract (§ 15) in `docs/app-contract/vectors.json`:
//! made here from fixed keys and claims, and checked against the
//! verification of this crate.
//!
//! Ed25519 is deterministic, so the file must equal what this test makes.
//! After a deliberate change, write it anew with
//! `ROEDY_UPDATE_VECTORS=1 cargo test -p roedy-hub-contract --test vectors`.

use std::path::PathBuf;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use roedy_hub_contract::{Error, Event, Expected, Identity, Jwk, Jwks, SigningKey, Typ, verify};
use serde::Serialize;
use serde_json::{Value, json};

const ISSUER: &str = "https://app.example.com";
const AUDIENCE: &str = "mail";
/// The key in the JWKS: RFC 8037, appendix A.1 – the same public key as the
/// example in contract § 4.4.
const KID: &str = "01JDX3M8Q4V6S2T0R9P7N5K3H1";
const SEED: &str = "nWGxne_9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A";
const RFC_8037_X: &str = "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo";
/// A second key that is not in the JWKS.
const OTHER_KID: &str = "01JDX3M8Q4V6S2T0R9P7N5K3H2";
const OTHER_SEED: [u8; 32] = [0x2a; 32];

/// Times of the examples in contract § 15.
const IDENTITY_IAT: i64 = 1_795_600_123;
const IDENTITY_NOW: i64 = 1_795_600_150;
const EVENT_IAT: i64 = 1_795_600_200;
const EVENT_NOW: i64 = 1_795_600_210;

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/app-contract/vectors.json")
}

fn key() -> SigningKey {
    let seed: [u8; 32] = URL_SAFE_NO_PAD.decode(SEED).unwrap().try_into().unwrap();
    SigningKey::from_seed(KID, &seed).unwrap()
}

fn other_key() -> SigningKey {
    SigningKey::from_seed(OTHER_KID, &OTHER_SEED).unwrap()
}

/// The identity of contract § 15.
fn identity() -> Identity {
    Identity {
        iss: ISSUER.into(),
        aud: AUDIENCE.into(),
        sub: "01JDX2Q7B5W3Y1Z9X7V5T3R1P0".into(),
        username: "alex".into(),
        email: "alex@example.com".into(),
        name: "Alex Example".into(),
        role: "user".into(),
        app_role: "user".into(),
        locale: "de".into(),
        sid: "01JDX3A1C3E5G7J9K1M3N5P7Q9".into(),
        auth_time: 1_795_600_000,
        iat: IDENTITY_IAT,
        exp: IDENTITY_IAT + 60,
    }
}

/// The event of contract § 15.
fn event() -> Event {
    Event {
        iss: ISSUER.into(),
        aud: AUDIENCE.into(),
        jti: "01JDX5R2T4V6X8Z0B2D4F6H8J0".into(),
        seq: 42,
        kind: "session.revoked".into(),
        data: json!({
            "sub": "01JDX2Q7B5W3Y1Z9X7V5T3R1P0",
            "sid": "01JDX3A1C3E5G7J9K1M3N5P7Q9",
            "reason": "logout",
        }),
        iat: EVENT_IAT,
        exp: EVENT_IAT + 300,
    }
}

fn header(typ: Typ, kid: &str) -> Value {
    json!({ "alg": "EdDSA", "typ": typ.as_str(), "kid": kid })
}

fn identity_json() -> Value {
    serde_json::to_value(identity()).unwrap()
}

/// `header` and `claims` signed with `key`, exactly as given.
fn raw(key: &SigningKey, header: &Value, claims: &Value) -> String {
    key.sign_raw(header.to_string().as_bytes(), claims.to_string().as_bytes())
}

/// The identity with one claim changed, properly signed.
fn identity_with(change: impl FnOnce(&mut Value)) -> String {
    let mut claims = identity_json();
    change(&mut claims);
    raw(&key(), &header(Typ::Identity, KID), &claims)
}

/// The identity with a changed header, properly signed.
fn identity_header(change: impl FnOnce(&mut Value)) -> String {
    let mut header = header(Typ::Identity, KID);
    change(&mut header);
    raw(&key(), &header, &identity_json())
}

fn segments(token: &str) -> [String; 3] {
    let parts: Vec<String> = token.split('.').map(str::to_owned).collect();
    parts.try_into().unwrap()
}

struct Case {
    name: &'static str,
    description: &'static str,
    typ: Typ,
    now: i64,
    token: String,
    /// The claims for a valid token, else the error.
    result: Result<Value, Error>,
}

impl Case {
    fn identity(
        name: &'static str,
        description: &'static str,
        token: String,
        error: Error,
    ) -> Self {
        Self {
            name,
            description,
            typ: Typ::Identity,
            now: IDENTITY_NOW,
            token,
            result: Err(error),
        }
    }
}

fn cases() -> Vec<Case> {
    let valid = key().sign(&identity()).unwrap();
    let [h, p, s] = segments(&valid);
    let event = key().sign(&event()).unwrap();

    let mut forged = URL_SAFE_NO_PAD.decode(&s).unwrap();
    forged[10] ^= 0x01;
    let mut tampered = identity_json();
    tampered["app_role"] = json!("admin");

    // Key confusion: HS256 with the public key as the HMAC secret.
    let hs256_header = URL_SAFE_NO_PAD
        .encode(json!({ "alg": "HS256", "typ": "roedy-id+jwt", "kid": KID }).to_string());
    let hs256_input = format!("{hs256_header}.{p}");
    let public = key().public_key();
    let mac = ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &public),
        hs256_input.as_bytes(),
    );
    let none_header = URL_SAFE_NO_PAD
        .encode(json!({ "alg": "none", "typ": "roedy-id+jwt", "kid": KID }).to_string());

    let mut unknown_claim = identity_json();
    unknown_claim["future_claim"] = json!({ "nested": true });

    vec![
        Case {
            name: "identity",
            description: "The identity of contract § 15.",
            typ: Typ::Identity,
            now: IDENTITY_NOW,
            token: valid.clone(),
            result: Ok(identity_json()),
        },
        Case {
            name: "identity-unknown-claim",
            description: "Unknown claims are ignored.",
            typ: Typ::Identity,
            now: IDENTITY_NOW,
            token: raw(&key(), &header(Typ::Identity, KID), &unknown_claim),
            result: Ok(unknown_claim),
        },
        Case {
            name: "identity-expiring-within-leeway",
            description: "now = exp + 9: still valid with the leeway of 10 s.",
            typ: Typ::Identity,
            now: IDENTITY_IAT + 60 + 9,
            token: valid.clone(),
            result: Ok(identity_json()),
        },
        Case {
            name: "identity-iat-within-leeway",
            description: "iat = now + 10: valid with the leeway of 10 s.",
            typ: Typ::Identity,
            now: IDENTITY_IAT - 10,
            token: valid.clone(),
            result: Ok(identity_json()),
        },
        Case {
            name: "event",
            description: "The event of contract § 15, verified as an event.",
            typ: Typ::Event,
            now: EVENT_NOW,
            token: event.clone(),
            result: Ok(serde_json::to_value(self::event()).unwrap()),
        },
        Case {
            name: "identity-expired",
            description: "now = exp + 10.",
            typ: Typ::Identity,
            now: IDENTITY_IAT + 60 + 10,
            token: valid.clone(),
            result: Err(Error::Expired),
        },
        Case {
            name: "identity-iat-in-future",
            description: "iat = now + 11.",
            typ: Typ::Identity,
            now: IDENTITY_IAT - 11,
            token: valid.clone(),
            result: Err(Error::IssuedInFuture),
        },
        Case::identity(
            "identity-lifetime-61",
            "exp − iat = 61 s, longer than an identity may live.",
            identity_with(|c| c["exp"] = json!(IDENTITY_IAT + 61)),
            Error::Lifetime,
        ),
        Case {
            name: "event-lifetime-301",
            description: "exp − iat = 301 s, longer than an event may live.",
            typ: Typ::Event,
            now: EVENT_NOW,
            token: {
                let mut claims = serde_json::to_value(self::event()).unwrap();
                claims["exp"] = json!(EVENT_IAT + 301);
                raw(&key(), &header(Typ::Event, KID), &claims)
            },
            result: Err(Error::Lifetime),
        },
        Case::identity(
            "forged-signature",
            "One bit of the signature flipped.",
            format!("{h}.{p}.{}", URL_SAFE_NO_PAD.encode(&forged)),
            Error::Signature,
        ),
        Case::identity(
            "tampered-claims",
            "app_role changed to admin, the signature of the original kept.",
            format!("{h}.{}.{s}", URL_SAFE_NO_PAD.encode(tampered.to_string())),
            Error::Signature,
        ),
        Case::identity(
            "wrong-key",
            "Signed by a key that is not in the JWKS, with the kid of one that is.",
            raw(&other_key(), &header(Typ::Identity, KID), &identity_json()),
            Error::Signature,
        ),
        Case::identity(
            "alg-none",
            "alg none without a signature.",
            format!("{none_header}.{p}."),
            Error::Algorithm,
        ),
        Case::identity(
            "alg-hs256-key-confusion",
            "alg HS256 with the public key (32 bytes) as the HMAC secret.",
            format!("{hs256_input}.{}", URL_SAFE_NO_PAD.encode(mac.as_ref())),
            Error::Algorithm,
        ),
        Case::identity(
            "alg-missing",
            "No alg in the header, otherwise properly signed.",
            identity_header(|h| {
                h.as_object_mut().unwrap().remove("alg");
            }),
            Error::Algorithm,
        ),
        Case::identity(
            "event-as-identity",
            "A valid event presented as an identity.",
            event.clone(),
            Error::Type,
        ),
        Case {
            name: "identity-as-event",
            description: "A valid identity presented as an event.",
            typ: Typ::Event,
            now: IDENTITY_NOW,
            token: valid.clone(),
            result: Err(Error::Type),
        },
        Case::identity(
            "typ-jwt",
            "typ JWT instead of roedy-id+jwt.",
            identity_header(|h| h["typ"] = json!("JWT")),
            Error::Type,
        ),
        Case::identity(
            "unknown-kid",
            "Signed by a key whose kid is not in the JWKS: refresh the JWKS, then reject.",
            other_key().sign(&identity()).unwrap(),
            Error::UnknownKey,
        ),
        Case::identity(
            "kid-missing",
            "No kid in the header.",
            identity_header(|h| {
                h.as_object_mut().unwrap().remove("kid");
            }),
            Error::Malformed,
        ),
        Case::identity(
            "crit",
            "A crit header member: the contract has no extensions.",
            identity_header(|h| h["crit"] = json!(["exp"])),
            Error::Malformed,
        ),
        Case::identity(
            "two-segments",
            "Header and claims without a signature segment.",
            format!("{h}.{p}"),
            Error::Malformed,
        ),
        Case::identity(
            "padding",
            "Base64url with padding.",
            format!("{h}=.{p}.{s}"),
            Error::Malformed,
        ),
        Case::identity(
            "wrong-aud",
            "aud is another app.",
            identity_with(|c| c["aud"] = json!("contacts")),
            Error::Audience,
        ),
        Case::identity(
            "aud-array",
            "aud as an array: always a string in the contract.",
            identity_with(|c| c["aud"] = json!([AUDIENCE])),
            Error::Claims,
        ),
        Case::identity(
            "wrong-iss",
            "iss is another hub.",
            identity_with(|c| c["iss"] = json!("https://other.example.com")),
            Error::Issuer,
        ),
    ]
}

/// The file, in a fixed order of members (`serde_json::Value` sorts them).
#[derive(Serialize)]
struct Document {
    description: &'static str,
    issuer: &'static str,
    audience: &'static str,
    jwks: Jwks,
    private_keys: Vec<PrivateJwk>,
    cases: Vec<CaseOut>,
}

#[derive(Serialize)]
struct PrivateJwk {
    #[serde(flatten)]
    public: Jwk,
    d: String,
}

#[derive(Serialize)]
struct CaseOut {
    name: &'static str,
    description: &'static str,
    typ: &'static str,
    now: i64,
    token: String,
    valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    claims: Option<Claims>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}

/// Claims in the contract's order where possible.
#[derive(Serialize)]
#[serde(untagged)]
enum Claims {
    Identity(Identity),
    Event(Event),
    Other(Value),
}

impl Claims {
    fn of(value: Value) -> Self {
        if let Ok(identity) = serde_json::from_value::<Identity>(value.clone())
            && serde_json::to_value(&identity).unwrap() == value
        {
            return Self::Identity(identity);
        }
        if let Ok(event) = serde_json::from_value::<Event>(value.clone())
            && serde_json::to_value(&event).unwrap() == value
        {
            return Self::Event(event);
        }
        Self::Other(value)
    }
}

const DESCRIPTION: &str = "Test vectors of the Roedy Hub app contract, version 1 \
    (docs/app-contract.md § 4 and § 10.1). For each case, verify `token` as `typ` against \
    `jwks` with `issuer`, `audience` and `now` (Unix seconds): a valid token gives `claims`; \
    an invalid one must be rejected, and `error` names the first check that fails in the \
    order of § 4.3. The private keys are published test keys: never use them for anything \
    else. Made by crates/contract/tests/vectors.rs.";

fn document() -> Document {
    let seed = URL_SAFE_NO_PAD.decode(SEED).unwrap();
    let private = |key: &SigningKey, seed: &[u8]| PrivateJwk {
        public: key.jwk(),
        d: URL_SAFE_NO_PAD.encode(seed),
    };
    let cases = cases()
        .into_iter()
        .map(|case| CaseOut {
            name: case.name,
            description: case.description,
            typ: case.typ.as_str(),
            now: case.now,
            token: case.token,
            valid: case.result.is_ok(),
            error: case.result.as_ref().err().map(|e| e.code()),
            claims: case.result.ok().map(Claims::of),
        })
        .collect();
    Document {
        description: DESCRIPTION,
        issuer: ISSUER,
        audience: AUDIENCE,
        jwks: Jwks {
            keys: vec![key().jwk()],
        },
        private_keys: vec![private(&key(), &seed), private(&other_key(), &OTHER_SEED)],
        cases,
    }
}

#[test]
fn the_published_file_is_up_to_date() {
    let expected = format!("{}\n", serde_json::to_string_pretty(&document()).unwrap());
    if std::env::var_os("ROEDY_UPDATE_VECTORS").is_some() {
        std::fs::write(path(), &expected).unwrap();
    }
    let published = std::fs::read_to_string(path()).unwrap_or_default();
    assert!(
        published == expected,
        "docs/app-contract/vectors.json is outdated; write it anew with \
         ROEDY_UPDATE_VECTORS=1 cargo test -p roedy-hub-contract --test vectors"
    );
}

#[test]
fn the_test_key_is_the_one_of_rfc_8037() {
    assert_eq!(key().jwk().x.as_deref(), Some(RFC_8037_X));
}

/// Reads the published file like an app would and checks every case.
#[test]
fn every_case_verifies_as_published() {
    let text = std::fs::read_to_string(path()).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    let jwks: Jwks = serde_json::from_value(doc["jwks"].clone()).unwrap();
    let issuer = doc["issuer"].as_str().unwrap();
    let audience = doc["audience"].as_str().unwrap();
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 20);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let token = case["token"].as_str().unwrap();
        let expected = Expected {
            issuer,
            audience,
            now: case["now"].as_i64().unwrap(),
        };
        let result = match Typ::parse(case["typ"].as_str().unwrap()).unwrap() {
            Typ::Identity => verify::<Identity>(token, &jwks, &expected)
                .map(|claims| serde_json::to_value(claims).unwrap()),
            Typ::Event => verify::<Event>(token, &jwks, &expected)
                .map(|claims| serde_json::to_value(claims).unwrap()),
        };
        if case["valid"] == json!(true) {
            let mut claims = case["claims"].clone();
            // Unknown claims are dropped when read.
            claims.as_object_mut().unwrap().remove("future_claim");
            assert_eq!(result, Ok(claims), "{name}");
        } else {
            let code = result.map(|_| "valid").unwrap_or_else(Error::code);
            assert_eq!(code, case["error"].as_str().unwrap(), "{name}");
        }
    }
}

/// The private keys in the file belong to the published and the unknown key.
#[test]
fn private_keys_match_their_public_keys() {
    let text = std::fs::read_to_string(path()).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    for jwk in doc["private_keys"].as_array().unwrap() {
        let seed: [u8; 32] = URL_SAFE_NO_PAD
            .decode(jwk["d"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let key = SigningKey::from_seed(jwk["kid"].as_str().unwrap(), &seed).unwrap();
        assert_eq!(key.jwk().x.as_deref(), jwk["x"].as_str());
    }
}
