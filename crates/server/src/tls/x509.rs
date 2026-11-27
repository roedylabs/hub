//! The X.509 structures the hub writes and reads, in DER ([`der`]):
//!
//! - the CSR for an order (RFC 2986, PKCS #10) with the domain as
//!   `subjectAltName`;
//! - certificates ([`CertificateParams`]): the self-signed TLS-ALPN-01
//!   challenge certificate (RFC 8737) and, in the tests, the fake CA's;
//! - [`CertInfo`]: validity, serial, names and key identifiers of a
//!   certificate, for renewal and ARI (RFC 9773).
//!
//! Only ECDSA P-256 with SHA-256 is written; reading does not check
//! signatures except where said.

use std::net::IpAddr;

use anyhow::{bail, ensure};
use jiff::Timestamp;
use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};

use super::{
    der::{self, Reader},
    key::P256Key,
};

/// Object identifiers in use.
pub mod oids {
    use super::der::oid;

    pub const EC_PUBLIC_KEY: &[u64] = &[1, 2, 840, 10045, 2, 1];
    pub const PRIME256V1: &[u64] = &[1, 2, 840, 10045, 3, 1, 7];
    pub const ECDSA_WITH_SHA256: &[u64] = &[1, 2, 840, 10045, 4, 3, 2];
    pub const COMMON_NAME: &[u64] = &[2, 5, 4, 3];
    pub const EXTENSION_REQUEST: &[u64] = &[1, 2, 840, 113549, 1, 9, 14];
    pub const SUBJECT_KEY_ID: &[u64] = &[2, 5, 29, 14];
    pub const SUBJECT_ALT_NAME: &[u64] = &[2, 5, 29, 17];
    pub const BASIC_CONSTRAINTS: &[u64] = &[2, 5, 29, 19];
    pub const AUTHORITY_KEY_ID: &[u64] = &[2, 5, 29, 35];
    pub const EXT_KEY_USAGE: &[u64] = &[2, 5, 29, 37];
    pub const SERVER_AUTH: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 3, 1];
    /// `id-pe-acmeIdentifier` (RFC 8737 § 6.1).
    pub const ACME_IDENTIFIER: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 1, 31];

    pub fn ec_public_key() -> Vec<u8> {
        oid(EC_PUBLIC_KEY)
    }

    pub fn prime256v1() -> Vec<u8> {
        oid(PRIME256V1)
    }
}

/// `AlgorithmIdentifier` of `ecdsa-with-SHA256` (no parameters).
fn ecdsa_sha256() -> Vec<u8> {
    der::sequence(&[&der::oid(oids::ECDSA_WITH_SHA256)])
}

/// A `Name` with only a common name, or empty.
pub fn name(common_name: Option<&str>) -> Vec<u8> {
    match common_name {
        Some(cn) => {
            let attribute = der::sequence(&[
                &der::oid(oids::COMMON_NAME),
                &der::tlv(der::UTF8_STRING, cn.as_bytes()),
            ]);
            der::sequence(&[&der::constructed(der::SET, &[&attribute])])
        }
        None => der::sequence(&[]),
    }
}

/// One certificate extension.
#[derive(Debug, Clone)]
pub struct Extension {
    pub oid: &'static [u64],
    pub critical: bool,
    /// The DER inside `extnValue`.
    pub value: Vec<u8>,
}

impl Extension {
    fn encode(&self) -> Vec<u8> {
        let critical = if self.critical {
            der::boolean(true)
        } else {
            Vec::new()
        };
        der::sequence(&[
            &der::oid(self.oid),
            &critical,
            &der::tlv(der::OCTET_STRING, &self.value),
        ])
    }

    /// `subjectAltName` with DNS names.
    pub fn dns_names(names: &[&str]) -> Self {
        Self::alt_names(names, &[])
    }

    /// `subjectAltName` with DNS names and IP addresses.
    pub fn alt_names(dns: &[&str], ips: &[IpAddr]) -> Self {
        let names: Vec<Vec<u8>> = dns
            .iter()
            .map(|name| der::tlv(der::context_primitive(2), name.as_bytes()))
            .chain(ips.iter().map(|ip| match ip {
                IpAddr::V4(ip) => der::tlv(der::context_primitive(7), &ip.octets()),
                IpAddr::V6(ip) => der::tlv(der::context_primitive(7), &ip.octets()),
            }))
            .collect();
        let refs: Vec<&[u8]> = names.iter().map(Vec::as_slice).collect();
        Self {
            oid: oids::SUBJECT_ALT_NAME,
            critical: false,
            value: der::sequence(&refs),
        }
    }

    /// The critical `acmeIdentifier` with the SHA-256 of the key
    /// authorization (RFC 8737 § 3).
    pub fn acme_identifier(digest: &[u8; 32]) -> Self {
        Self {
            oid: oids::ACME_IDENTIFIER,
            critical: true,
            value: der::tlv(der::OCTET_STRING, digest),
        }
    }

    /// `basicConstraints` of a CA.
    pub fn ca() -> Self {
        Self {
            oid: oids::BASIC_CONSTRAINTS,
            critical: true,
            value: der::sequence(&[&der::boolean(true)]),
        }
    }

    /// `extKeyUsage` for TLS servers.
    pub fn server_auth() -> Self {
        Self {
            oid: oids::EXT_KEY_USAGE,
            critical: false,
            value: der::sequence(&[&der::oid(oids::SERVER_AUTH)]),
        }
    }

    pub fn subject_key_id(id: &[u8]) -> Self {
        Self {
            oid: oids::SUBJECT_KEY_ID,
            critical: false,
            value: der::tlv(der::OCTET_STRING, id),
        }
    }

    pub fn authority_key_id(id: &[u8]) -> Self {
        Self {
            oid: oids::AUTHORITY_KEY_ID,
            critical: false,
            value: der::sequence(&[&der::tlv(der::context_primitive(0), id)]),
        }
    }
}

/// A certificate to sign.
#[derive(Debug, Clone)]
pub struct CertificateParams {
    /// Big-endian; made positive.
    pub serial: Vec<u8>,
    /// `Name` DER ([`name`]).
    pub issuer: Vec<u8>,
    pub subject: Vec<u8>,
    pub not_before: Timestamp,
    pub not_after: Timestamp,
    /// `SubjectPublicKeyInfo` DER of the certified key.
    pub spki: Vec<u8>,
    pub extensions: Vec<Extension>,
}

impl CertificateParams {
    /// The certificate in DER, signed by `issuer_key`.
    pub fn sign(&self, issuer_key: &P256Key) -> anyhow::Result<Vec<u8>> {
        let extensions: Vec<Vec<u8>> = self.extensions.iter().map(Extension::encode).collect();
        let refs: Vec<&[u8]> = extensions.iter().map(Vec::as_slice).collect();
        let tbs = der::sequence(&[
            &der::constructed(der::context(0), &[&der::unsigned(&[2])]),
            &der::unsigned(&self.serial),
            &ecdsa_sha256(),
            &self.issuer,
            &der::sequence(&[&der::time(self.not_before), &der::time(self.not_after)]),
            &self.subject,
            &self.spki,
            &der::constructed(der::context(3), &[&der::sequence(&refs)]),
        ]);
        let signature = issuer_key.sign_der(&tbs)?;
        Ok(der::sequence(&[
            &tbs,
            &ecdsa_sha256(),
            &der::bit_string(&signature),
        ]))
    }
}

/// The TLS-ALPN-01 certificate for `domain` (RFC 8737 § 3): self-signed,
/// the domain as its only `subjectAltName` and the critical
/// `acmeIdentifier`, valid from a day ago for a week. Issuer and subject
/// carry a common name, as RFC 5280 wants of a self-signed certificate.
pub fn alpn_challenge_certificate(
    domain: &str,
    digest: &[u8; 32],
    key: &P256Key,
    now: Timestamp,
) -> anyhow::Result<Vec<u8>> {
    let day = jiff::SignedDuration::from_hours(24);
    let dn = name(Some(if domain.len() <= 64 {
        domain
    } else {
        "ACME TLS-ALPN-01"
    }));
    CertificateParams {
        serial: crate::random::bytes::<16>()?.to_vec(),
        issuer: dn.clone(),
        subject: dn,
        not_before: now - day,
        not_after: now + day * 7,
        spki: key.spki(),
        extensions: vec![
            Extension::dns_names(&[domain]),
            Extension::acme_identifier(digest),
        ],
    }
    .sign(key)
}

/// The CSR for `domain` (RFC 2986): the domain as common name (if it fits
/// the 64 characters X.520 allows) and as the `subjectAltName`, which is what
/// the CA certifies (RFC 8555 § 7.4).
pub fn csr(domain: &str, key: &P256Key) -> anyhow::Result<Vec<u8>> {
    let extensions = der::sequence(&[&Extension::dns_names(&[domain]).encode()]);
    let attribute = der::sequence(&[
        &der::oid(oids::EXTENSION_REQUEST),
        &der::constructed(der::SET, &[&extensions]),
    ]);
    let info = der::sequence(&[
        &der::unsigned(&[0]),
        &name((domain.len() <= 64).then_some(domain)),
        &key.spki(),
        &der::constructed(der::context(0), &[&attribute]),
    ]);
    let signature = key.sign_der(&info)?;
    Ok(der::sequence(&[
        &info,
        &ecdsa_sha256(),
        &der::bit_string(&signature),
    ]))
}

/// What a CSR asks for, read back and its signature checked: the P-256
/// public key (uncompressed point) and the requested DNS names. For the
/// tests' fake CA and the unit tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsrInfo {
    pub spki: Vec<u8>,
    pub public_point: Vec<u8>,
    pub dns_names: Vec<String>,
}

impl CsrInfo {
    pub fn parse(csr: &[u8]) -> anyhow::Result<Self> {
        let mut outer = Reader::new(csr);
        let request = outer.expect(der::SEQUENCE)?;
        outer.finish()?;
        let mut request = request.reader();
        let info = request.expect(der::SEQUENCE)?;
        let algorithm = request.expect(der::SEQUENCE)?;
        ensure!(
            algorithm.raw == ecdsa_sha256(),
            "CSR: not ecdsa-with-SHA256"
        );
        let signature = der::bit_string_bytes(request.expect(der::BIT_STRING)?.content)?;
        request.finish()?;

        let mut fields = info.reader();
        ensure!(
            fields.expect(der::INTEGER)?.content == [0],
            "CSR: version is not 1"
        );
        fields.expect(der::SEQUENCE)?;
        let spki = fields.expect(der::SEQUENCE)?;
        let public_point = p256_point(spki.raw)?;
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &public_point)
            .verify(info.raw, signature)
            .map_err(|_| anyhow::anyhow!("CSR: bad signature"))?;

        let mut dns_names = Vec::new();
        if let Some(attributes) = fields.optional(der::context(0))? {
            let mut attributes = attributes.reader();
            while !attributes.is_empty() {
                let mut attribute = attributes.expect(der::SEQUENCE)?.reader();
                let id = attribute.expect(der::OID)?;
                let mut values = attribute.expect(der::SET)?.reader();
                if id.raw != der::oid(oids::EXTENSION_REQUEST) {
                    continue;
                }
                let mut extensions = values.expect(der::SEQUENCE)?.reader();
                while !extensions.is_empty() {
                    let (oid, _, value) = extension(&mut extensions)?;
                    if oid == der::oid(oids::SUBJECT_ALT_NAME) {
                        dns_names.extend(alt_names(value)?.0);
                    }
                }
            }
        }
        fields.finish()?;
        Ok(Self {
            spki: spki.raw.to_vec(),
            public_point,
            dns_names,
        })
    }
}

/// The uncompressed P-256 point of a `SubjectPublicKeyInfo`.
pub fn p256_point(spki: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut outer = Reader::new(spki);
    let mut spki = outer.expect(der::SEQUENCE)?.reader();
    outer.finish()?;
    let algorithm = spki.expect(der::SEQUENCE)?;
    ensure!(
        algorithm.raw == der::sequence(&[&oids::ec_public_key(), &oids::prime256v1()]),
        "not a P-256 key"
    );
    let point = der::bit_string_bytes(spki.expect(der::BIT_STRING)?.content)?;
    spki.finish()?;
    ensure!(
        point.len() == 65 && point[0] == 4,
        "not an uncompressed point"
    );
    Ok(point.to_vec())
}

/// One `Extension`: its OID (whole encoding), criticality and the DER in
/// `extnValue`.
fn extension<'a>(reader: &mut Reader<'a>) -> anyhow::Result<(&'a [u8], bool, &'a [u8])> {
    let mut fields = reader.expect(der::SEQUENCE)?.reader();
    let oid = fields.expect(der::OID)?.raw;
    let critical = match fields.optional(der::BOOLEAN)? {
        Some(value) => value.content == [0xff],
        None => false,
    };
    let value = fields.expect(der::OCTET_STRING)?.content;
    fields.finish()?;
    Ok((oid, critical, value))
}

/// The `dNSName`s and `iPAddress`es of a `subjectAltName` value; other
/// kinds are skipped.
fn alt_names(value: &[u8]) -> anyhow::Result<(Vec<String>, Vec<IpAddr>)> {
    let mut outer = Reader::new(value);
    let mut names = outer.expect(der::SEQUENCE)?.reader();
    outer.finish()?;
    let (mut dns, mut ips) = (Vec::new(), Vec::new());
    while !names.is_empty() {
        let name = names.read()?;
        if name.tag == der::context_primitive(2) {
            dns.push(String::from_utf8(name.content.to_vec())?);
        } else if name.tag == der::context_primitive(7) {
            if let Ok(octets) = <[u8; 4]>::try_from(name.content) {
                ips.push(IpAddr::from(octets));
            } else if let Ok(octets) = <[u8; 16]>::try_from(name.content) {
                ips.push(IpAddr::from(octets));
            }
        }
    }
    Ok((dns, ips))
}

/// What the hub reads from a certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    /// The serial's content octets as encoded (with a leading zero if there
    /// is one), as ARI's certificate ID uses them.
    pub serial: Vec<u8>,
    pub not_before: Timestamp,
    pub not_after: Timestamp,
    pub dns_names: Vec<String>,
    /// `iPAddress` names.
    pub ip_addresses: Vec<IpAddr>,
    /// `keyIdentifier` of the authority key identifier.
    pub authority_key_id: Option<Vec<u8>>,
    pub subject_key_id: Option<Vec<u8>>,
    /// Content of `acmeIdentifier`, if present, and whether it was critical.
    pub acme_identifier: Option<(Vec<u8>, bool)>,
    pub spki: Vec<u8>,
    /// The signed part and the signature, for checks against an issuer.
    pub tbs: Vec<u8>,
    pub signature: Vec<u8>,
}

impl CertInfo {
    pub fn parse(certificate: &[u8]) -> anyhow::Result<Self> {
        let mut outer = Reader::new(certificate);
        let mut cert = outer.expect(der::SEQUENCE)?.reader();
        outer.finish()?;
        let tbs = cert.expect(der::SEQUENCE)?;
        cert.expect(der::SEQUENCE)?;
        let signature = der::bit_string_bytes(cert.expect(der::BIT_STRING)?.content)?;
        cert.finish()?;

        let mut fields = tbs.reader();
        let version = fields.expect(der::context(0))?;
        ensure!(
            version.reader().expect(der::INTEGER)?.content == [2],
            "certificate: not version 3"
        );
        let serial = fields.expect(der::INTEGER)?.content.to_vec();
        fields.expect(der::SEQUENCE)?;
        fields.expect(der::SEQUENCE)?;
        let mut validity = fields.expect(der::SEQUENCE)?.reader();
        let mut time = || -> anyhow::Result<Timestamp> {
            let element = validity.read()?;
            der::parse_time(element.tag, element.content)
        };
        let (not_before, not_after) = (time()?, time()?);
        fields.expect(der::SEQUENCE)?;
        let spki = fields.expect(der::SEQUENCE)?.raw.to_vec();
        let mut info = Self {
            serial,
            not_before,
            not_after,
            dns_names: Vec::new(),
            ip_addresses: Vec::new(),
            authority_key_id: None,
            subject_key_id: None,
            acme_identifier: None,
            spki,
            tbs: tbs.raw.to_vec(),
            signature: signature.to_vec(),
        };
        // Issuer and subject unique IDs ([1], [2]) are skipped.
        while let Some(tag) = fields.peek_tag() {
            let element = fields.read()?;
            if tag != der::context(3) {
                continue;
            }
            let mut wrapper = element.reader();
            let mut extensions = wrapper.expect(der::SEQUENCE)?.reader();
            wrapper.finish()?;
            while !extensions.is_empty() {
                let (oid, critical, value) = extension(&mut extensions)?;
                info.read_extension(oid, critical, value)?;
            }
        }
        Ok(info)
    }

    fn read_extension(&mut self, oid: &[u8], critical: bool, value: &[u8]) -> anyhow::Result<()> {
        if oid == der::oid(oids::SUBJECT_ALT_NAME) {
            (self.dns_names, self.ip_addresses) = alt_names(value)?;
        } else if oid == der::oid(oids::AUTHORITY_KEY_ID) {
            let mut outer = Reader::new(value);
            let mut fields = outer.expect(der::SEQUENCE)?.reader();
            self.authority_key_id = fields
                .optional(der::context_primitive(0))?
                .map(|id| id.content.to_vec());
        } else if oid == der::oid(oids::SUBJECT_KEY_ID) {
            self.subject_key_id = Some(
                Reader::new(value)
                    .expect(der::OCTET_STRING)?
                    .content
                    .to_vec(),
            );
        } else if oid == der::oid(oids::ACME_IDENTIFIER) {
            let digest = Reader::new(value)
                .expect(der::OCTET_STRING)?
                .content
                .to_vec();
            self.acme_identifier = Some((digest, critical));
        }
        Ok(())
    }

    /// Whether `issuer_spki` (a P-256 key) signed this certificate.
    pub fn signed_by(&self, issuer_spki: &[u8]) -> anyhow::Result<()> {
        let point = p256_point(issuer_spki)?;
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &point)
            .verify(&self.tbs, &self.signature)
            .map_err(|_| anyhow::anyhow!("certificate: bad signature"))
    }

    /// The ARI certificate ID (RFC 9773 § 4.1): base64url of the authority
    /// key identifier and of the serial, joined by a dot.
    pub fn ari_id(&self) -> Option<String> {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        let aki = self.authority_key_id.as_ref()?;
        if self.serial.is_empty() {
            return None;
        }
        Some(format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(aki),
            URL_SAFE_NO_PAD.encode(&self.serial)
        ))
    }
}

/// Checks that `name` is a DNS name the hub can get a certificate for:
/// letters, digits and hyphens in labels of 1–63 characters, at least two
/// labels, 253 characters at most, no wildcard.
pub fn check_dns_name(name: &str) -> anyhow::Result<()> {
    let labels: Vec<&str> = name.split('.').collect();
    let valid = name.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        });
    if !valid {
        bail!("{name} is not a DNS name a certificate can be issued for");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use sha2::{Digest as _, Sha256};

    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn csr_parses_back_with_its_signature() {
        let key = P256Key::generate().unwrap();
        let csr = csr("hub.example.test", &key).unwrap();
        let info = CsrInfo::parse(&csr).unwrap();
        assert_eq!(info.dns_names, ["hub.example.test"]);
        assert_eq!(info.public_point, key.public_point());
        assert_eq!(info.spki, key.spki());

        // A long name goes without common name, still in the SAN.
        let long = format!("{}.example.test", "a".repeat(60));
        assert_eq!(
            CsrInfo::parse(&self::csr(&long, &key).unwrap())
                .unwrap()
                .dns_names,
            [long]
        );

        // A changed byte breaks the signature or the structure.
        let mut broken = csr.clone();
        let at = broken.len() / 2;
        broken[at] ^= 1;
        assert!(CsrInfo::parse(&broken).is_err());
    }

    #[test]
    fn alpn_certificate_has_the_critical_acme_identifier() {
        let key = P256Key::generate().unwrap();
        let digest: [u8; 32] = Sha256::digest(b"token.thumbprint").into();
        let now = at("2026-11-27T12:00:00Z");
        let der = alpn_challenge_certificate("hub.example.test", &digest, &key, now).unwrap();
        let info = CertInfo::parse(&der).unwrap();
        assert_eq!(info.dns_names, ["hub.example.test"]);
        assert_eq!(info.acme_identifier, Some((digest.to_vec(), true)));
        assert_eq!(info.not_before, at("2026-11-26T12:00:00Z"));
        assert_eq!(info.not_after, at("2026-12-04T12:00:00Z"));
        info.signed_by(&key.spki()).unwrap();
        assert!(
            info.signed_by(&P256Key::generate().unwrap().spki())
                .is_err()
        );
        // webpki (and so a browser) refuses it for the critical extension
        // it does not know – only the ACME server reads it.
        let cert = CertificateDer::from(der);
        assert!(matches!(
            webpki::EndEntityCert::try_from(&cert),
            Err(webpki::Error::UnsupportedCriticalExtension)
        ));
    }

    /// A CA and a leaf built with [`CertificateParams`] verify with webpki,
    /// as a browser would check the CA's chain.
    #[test]
    fn chain_verifies_with_webpki() {
        let ca_key = P256Key::generate().unwrap();
        let now = Timestamp::now();
        let day = jiff::SignedDuration::from_hours(24);
        let ca_name = name(Some("Fake CA"));
        let ca = CertificateParams {
            serial: vec![1],
            issuer: ca_name.clone(),
            subject: ca_name.clone(),
            not_before: now - day,
            not_after: now + day * 365,
            spki: ca_key.spki(),
            extensions: vec![Extension::ca(), Extension::subject_key_id(&[7; 20])],
        }
        .sign(&ca_key)
        .unwrap();
        let leaf_key = P256Key::generate().unwrap();
        let leaf = CertificateParams {
            serial: vec![0x80, 1, 2],
            issuer: ca_name,
            subject: name(None),
            not_before: now - day,
            not_after: now + day * 90,
            spki: leaf_key.spki(),
            extensions: vec![
                Extension::alt_names(
                    &["hub.example.test"],
                    &["192.0.2.7".parse().unwrap(), "2001:db8::1".parse().unwrap()],
                ),
                Extension::server_auth(),
                Extension::authority_key_id(&[7; 20]),
            ],
        }
        .sign(&ca_key)
        .unwrap();

        let info = CertInfo::parse(&leaf).unwrap();
        assert_eq!(info.serial, [0, 0x80, 1, 2]);
        assert_eq!(info.dns_names, ["hub.example.test"]);
        assert_eq!(
            info.ip_addresses,
            [
                "192.0.2.7".parse::<IpAddr>().unwrap(),
                "2001:db8::1".parse().unwrap()
            ]
        );
        assert_eq!(info.authority_key_id, Some(vec![7; 20]));
        assert_eq!(info.ari_id().unwrap(), "BwcHBwcHBwcHBwcHBwcHBwcHBwc.AIABAg");
        info.signed_by(&ca_key.spki()).unwrap();
        assert_eq!(
            CertInfo::parse(&ca).unwrap().subject_key_id,
            Some(vec![7; 20])
        );

        let ca = CertificateDer::from(ca);
        let anchor = webpki::anchor_from_trusted_cert(&ca).unwrap();
        let leaf = CertificateDer::from(leaf);
        let cert = webpki::EndEntityCert::try_from(&leaf).unwrap();
        let unix = UnixTime::since_unix_epoch(std::time::Duration::from_secs(
            u64::try_from(now.as_second()).unwrap(),
        ));
        cert.verify_for_usage(
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .all,
            &[anchor],
            &[],
            unix,
            webpki::KeyUsage::server_auth(),
            None,
            None,
        )
        .unwrap();
        cert.verify_is_valid_for_subject_name(&ServerName::try_from("hub.example.test").unwrap())
            .unwrap();
        assert!(
            cert.verify_is_valid_for_subject_name(
                &ServerName::try_from("other.example.test").unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn dns_names() {
        for good in ["hub.example.test", "a-b.example.com", "x1.y2.example"] {
            check_dns_name(good).unwrap();
        }
        for bad in [
            "localhost",
            "*.example.com",
            "-a.example.com",
            "a-.example.com",
            "a..example.com",
            "Hub.example.com",
            "192.0.2.1.example.com_",
        ] {
            assert!(check_dns_name(bad).is_err(), "{bad}");
        }
    }
}
