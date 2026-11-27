//! The JOSE parts of ACME (RFC 8555 § 6.2): flattened JWS with ES256, the
//! account key as a JWK and its thumbprint (RFC 7638) for the key
//! authorizations. Like the contract crate's JWS (`roedy-hub-contract`),
//! written directly on `ring`, `base64` and `serde_json`.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::key::P256Key;

pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The public JWK of a P-256 key: `{"crv","kty","x","y"}`.
pub fn jwk(key: &P256Key) -> Value {
    let (x, y) = key.coordinates();
    json!({ "crv": "P-256", "kty": "EC", "x": b64(x), "y": b64(y) })
}

/// RFC 7638 thumbprint over the required members, given as name and string
/// value: the members sorted by name, JSON without whitespace, SHA-256,
/// base64url.
pub fn thumbprint_of(members: &[(&str, &str)]) -> String {
    let mut members = members.to_vec();
    members.sort_by(|a, b| a.0.cmp(b.0));
    let object: serde_json::Map<String, Value> = members
        .into_iter()
        .map(|(name, value)| (name.to_owned(), Value::from(value)))
        .collect();
    // `serde_json::Map` keeps the keys sorted and writes no whitespace.
    let canonical = Value::Object(object).to_string();
    b64(&Sha256::digest(canonical.as_bytes()))
}

/// The thumbprint of the account key.
pub fn thumbprint(key: &P256Key) -> String {
    let (x, y) = key.coordinates();
    thumbprint_of(&[
        ("crv", "P-256"),
        ("kty", "EC"),
        ("x", &b64(x)),
        ("y", &b64(y)),
    ])
}

/// Who signs: a new account with its JWK, or a known one by its URL.
#[derive(Debug, Clone, Copy)]
pub enum Signer<'a> {
    Jwk,
    Kid(&'a str),
}

/// A flattened JWS (RFC 7515 § 7.2.2) over `payload` (`None` for
/// POST-as-GET, an empty payload) for `url` with `nonce`.
pub fn sign(
    key: &P256Key,
    signer: Signer<'_>,
    nonce: &str,
    url: &str,
    payload: Option<&Value>,
) -> anyhow::Result<Value> {
    let mut protected = json!({ "alg": "ES256", "nonce": nonce, "url": url });
    match signer {
        Signer::Jwk => protected["jwk"] = jwk(key),
        Signer::Kid(kid) => protected["kid"] = kid.into(),
    }
    let protected = b64(protected.to_string().as_bytes());
    let payload = match payload {
        Some(payload) => b64(payload.to_string().as_bytes()),
        None => String::new(),
    };
    let signature = key.sign_fixed(format!("{protected}.{payload}").as_bytes())?;
    Ok(json!({
        "protected": protected,
        "payload": payload,
        "signature": b64(&signature),
    }))
}

#[cfg(test)]
mod tests {
    use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};

    use super::*;

    /// RFC 7638 § 3.1: the RSA key from RFC 7517 appendix A.1.
    #[test]
    fn thumbprint_vector_of_rfc_7638() {
        let n = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tS\
                 oc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-6\
                 5YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNL\
                 yrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJ\
                 zKnqDKgw";
        assert_eq!(
            thumbprint_of(&[("kty", "RSA"), ("n", n), ("e", "AQAB")]),
            "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"
        );
    }

    /// The EC member order (`crv`, `kty`, `x`, `y`) of RFC 7638 § 3.2.
    #[test]
    fn ec_thumbprint_hashes_the_canonical_members() {
        let key = P256Key::generate().unwrap();
        let jwk = jwk(&key);
        let canonical = format!(
            r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
            jwk["x"].as_str().unwrap(),
            jwk["y"].as_str().unwrap()
        );
        assert_eq!(jwk.to_string(), canonical);
        assert_eq!(thumbprint(&key), b64(&Sha256::digest(canonical.as_bytes())));
    }

    #[test]
    fn signs_a_flattened_jws() {
        let key = P256Key::generate().unwrap();
        let url = "https://ca.example.test/acme/new-order";
        let jws = sign(
            &key,
            Signer::Kid("https://ca.example.test/acct/1"),
            "n0nce",
            url,
            Some(&json!({"a": 1})),
        )
        .unwrap();
        let protected = jws["protected"].as_str().unwrap();
        let header: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(protected).unwrap()).unwrap();
        assert_eq!(
            header,
            json!({ "alg": "ES256", "nonce": "n0nce", "url": url, "kid": "https://ca.example.test/acct/1" })
        );
        let payload = jws["payload"].as_str().unwrap();
        assert_eq!(URL_SAFE_NO_PAD.decode(payload).unwrap(), br#"{"a":1}"#);
        let signature = URL_SAFE_NO_PAD
            .decode(jws["signature"].as_str().unwrap())
            .unwrap();
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, key.public_point())
            .verify(format!("{protected}.{payload}").as_bytes(), &signature)
            .unwrap();

        // New account: the JWK instead of `kid`; POST-as-GET: empty payload.
        let jws = sign(&key, Signer::Jwk, "n", url, None).unwrap();
        let header: Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(jws["protected"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(header["jwk"], self::jwk(&key));
        assert!(header.get("kid").is_none());
        assert_eq!(jws["payload"], "");
    }
}
