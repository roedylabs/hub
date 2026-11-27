//! ECDSA P-256 keys with `ring`: the ACME account key (ES256, RFC 7518
//! § 3.4) and the certificates' keys (`ecdsa-with-SHA256` in X.509). Stored
//! as PKCS#8, which rustls loads as well.

use std::fmt;

use anyhow::anyhow;
use ring::{
    rand::SystemRandom,
    signature::{
        ECDSA_P256_SHA256_ASN1_SIGNING, ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _,
    },
};

use super::{der, x509::oids};

pub struct P256Key {
    pkcs8: Vec<u8>,
    /// Signatures as `r || s` (JWS).
    fixed: EcdsaKeyPair,
    /// Signatures as a DER `ECDSA-Sig-Value` (X.509).
    asn1: EcdsaKeyPair,
}

impl fmt::Debug for P256Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("P256Key(…)")
    }
}

impl P256Key {
    /// A new key from the system's random source.
    pub fn generate() -> anyhow::Result<Self> {
        let rng = SystemRandom::new();
        let document = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .map_err(|_| anyhow!("ECDSA key not generated"))?;
        Self::from_pkcs8(document.as_ref())
    }

    pub fn from_pkcs8(pkcs8: &[u8]) -> anyhow::Result<Self> {
        let rng = SystemRandom::new();
        let load = |alg| {
            EcdsaKeyPair::from_pkcs8(alg, pkcs8, &rng)
                .map_err(|e| anyhow!("not a P-256 key in PKCS#8: {e}"))
        };
        Ok(Self {
            pkcs8: pkcs8.to_vec(),
            fixed: load(&ECDSA_P256_SHA256_FIXED_SIGNING)?,
            asn1: load(&ECDSA_P256_SHA256_ASN1_SIGNING)?,
        })
    }

    pub fn pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }

    /// The public key as an uncompressed point (`04 || x || y`, 65 bytes).
    pub fn public_point(&self) -> &[u8] {
        self.fixed.public_key().as_ref()
    }

    /// `x` and `y` of the public key.
    pub fn coordinates(&self) -> (&[u8], &[u8]) {
        let point = &self.public_point()[1..];
        point.split_at(point.len() / 2)
    }

    /// `SubjectPublicKeyInfo` (RFC 5480).
    pub fn spki(&self) -> Vec<u8> {
        der::sequence(&[
            &der::sequence(&[&oids::ec_public_key(), &oids::prime256v1()]),
            &der::bit_string(self.public_point()),
        ])
    }

    /// ES256 signature, `r || s`.
    pub fn sign_fixed(&self, message: &[u8]) -> anyhow::Result<Vec<u8>> {
        let signature = self
            .fixed
            .sign(&SystemRandom::new(), message)
            .map_err(|_| anyhow!("ECDSA signature failed"))?;
        Ok(signature.as_ref().to_vec())
    }

    /// `ecdsa-with-SHA256` signature in DER.
    pub fn sign_der(&self, message: &[u8]) -> anyhow::Result<Vec<u8>> {
        let signature = self
            .asn1
            .sign(&SystemRandom::new(), message)
            .map_err(|_| anyhow!("ECDSA signature failed"))?;
        Ok(signature.as_ref().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use ring::signature::{ECDSA_P256_SHA256_ASN1, ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};

    use super::*;

    #[test]
    fn signs_in_both_formats_and_reloads() {
        let key = P256Key::generate().unwrap();
        assert_eq!(key.public_point().len(), 65);
        let (x, y) = key.coordinates();
        assert_eq!((x.len(), y.len()), (32, 32));
        let public = UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, key.public_point());
        let fixed = key.sign_fixed(b"message").unwrap();
        assert_eq!(fixed.len(), 64);
        public.verify(b"message", &fixed).unwrap();
        let asn1 = UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, key.public_point());
        asn1.verify(b"message", &key.sign_der(b"message").unwrap())
            .unwrap();

        let again = P256Key::from_pkcs8(key.pkcs8()).unwrap();
        assert_eq!(again.public_point(), key.public_point());
        assert!(P256Key::from_pkcs8(b"not a key").is_err());
        assert_eq!(format!("{key:?}"), "P256Key(…)");
    }
}
