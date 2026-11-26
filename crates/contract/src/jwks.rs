//! The hub's public signing keys as a JWK Set (contract § 4.4; RFC 7517, OKP
//! keys per RFC 8037).

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use crate::ALG;

/// A JWK Set: `{"keys": [...]}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

impl Jwks {
    /// The Ed25519 public key with this `kid`, if the set has one that is
    /// valid for the contract: `kty` `OKP`, `crv` `Ed25519`, `use` `sig`,
    /// `alg` `EdDSA` and 32 bytes in `x`. Keys of any other kind are skipped.
    pub fn find(&self, kid: &str) -> Option<[u8; 32]> {
        self.keys
            .iter()
            .filter(|jwk| jwk.kid.as_deref() == Some(kid))
            .find_map(Jwk::public_key)
    }
}

/// One key of a [`Jwks`]. Every field is optional when reading, so that a
/// set with keys of other kinds still parses; [`Jwks::find`] decides which
/// keys count.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Jwk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crv: Option<String>,
    /// Public key, base64url without padding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    pub use_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alg: Option<String>,
}

impl Jwk {
    /// The JWK of an Ed25519 public key, as the hub publishes it.
    pub fn ed25519(kid: &str, public: &[u8; 32]) -> Self {
        Self {
            kty: Some("OKP".to_owned()),
            crv: Some("Ed25519".to_owned()),
            x: Some(URL_SAFE_NO_PAD.encode(public)),
            kid: Some(kid.to_owned()),
            use_: Some("sig".to_owned()),
            alg: Some(ALG.to_owned()),
        }
    }

    /// The public key if this is a signing key the contract accepts.
    pub fn public_key(&self) -> Option<[u8; 32]> {
        let is = |field: &Option<String>, value: &str| field.as_deref() == Some(value);
        if !(is(&self.kty, "OKP")
            && is(&self.crv, "Ed25519")
            && is(&self.use_, "sig")
            && is(&self.alg, ALG))
        {
            return None;
        }
        URL_SAFE_NO_PAD
            .decode(self.x.as_deref()?)
            .ok()?
            .try_into()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_okp_keys_in_the_contracts_shape() {
        let jwks = Jwks {
            keys: vec![Jwk::ed25519("k1", &[7; 32])],
        };
        assert_eq!(
            serde_json::to_string(&jwks).unwrap(),
            r#"{"keys":[{"kty":"OKP","crv":"Ed25519","x":"BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc","kid":"k1","use":"sig","alg":"EdDSA"}]}"#
        );
        assert_eq!(jwks.find("k1"), Some([7; 32]));
        assert_eq!(jwks.find("k2"), None);
    }

    #[test]
    fn finds_only_keys_the_contract_accepts() {
        let valid = Jwk::ed25519("k", &[1; 32]);
        type Change = fn(&mut Jwk);
        let cases: [(&str, Change); 9] = [
            ("other kty", |k| k.kty = Some("EC".into())),
            ("other crv", |k| k.crv = Some("X25519".into())),
            ("enc key", |k| k.use_ = Some("enc".into())),
            ("without use", |k| k.use_ = None),
            ("other alg", |k| k.alg = Some("ES256".into())),
            ("without alg", |k| k.alg = None),
            ("short x", |k| k.x = Some(URL_SAFE_NO_PAD.encode([1; 31]))),
            ("padded x", |k| {
                k.x = Some(format!("{}=", k.x.as_ref().unwrap()))
            }),
            ("without x", |k| k.x = None),
        ];
        for (name, change) in cases {
            let mut jwk = valid.clone();
            change(&mut jwk);
            let jwks = Jwks { keys: vec![jwk] };
            assert_eq!(jwks.find("k"), None, "{name}");
        }
        // A later valid key with the same kid still counts.
        let mut other = valid.clone();
        other.use_ = Some("enc".into());
        let jwks = Jwks {
            keys: vec![other, valid],
        };
        assert_eq!(jwks.find("k"), Some([1; 32]));
    }

    #[test]
    fn reads_sets_with_foreign_keys_and_fields() {
        let jwks: Jwks = serde_json::from_str(
            r#"{"keys":[{"kty":"RSA","n":"AQAB","e":"AQAB","kid":"r"},
                {"kty":"OKP","crv":"Ed25519","x":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE",
                 "kid":"k","use":"sig","alg":"EdDSA","key_ops":["verify"]}],"extra":1}"#,
        )
        .unwrap();
        assert_eq!(jwks.keys.len(), 2);
        assert_eq!(jwks.find("r"), None);
        assert_eq!(jwks.find("k"), Some([1; 32]));
    }
}
