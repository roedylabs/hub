//! Minimal DER (ITU-T X.690) for what the hub writes and reads itself: the
//! CSR and the TLS-ALPN-01 certificate it builds, and the few fields it reads
//! from certificates (validity, serial, extensions). No ASN.1 library: the
//! structures are small and fixed, and the reader is strict (definite
//! lengths in their shortest form only).

use anyhow::{anyhow, bail, ensure};
use jiff::{Timestamp, civil::DateTime, tz::TimeZone};

pub const BOOLEAN: u8 = 0x01;
pub const INTEGER: u8 = 0x02;
pub const BIT_STRING: u8 = 0x03;
pub const OCTET_STRING: u8 = 0x04;
pub const NULL: u8 = 0x05;
pub const OID: u8 = 0x06;
pub const UTF8_STRING: u8 = 0x0c;
pub const PRINTABLE_STRING: u8 = 0x13;
pub const IA5_STRING: u8 = 0x16;
pub const UTC_TIME: u8 = 0x17;
pub const GENERALIZED_TIME: u8 = 0x18;
pub const SEQUENCE: u8 = 0x30;
pub const SET: u8 = 0x31;

/// `[n]` constructed (EXPLICIT tagging or an IMPLICIT constructed type).
pub const fn context(n: u8) -> u8 {
    0xa0 | n
}

/// `[n]` primitive (IMPLICIT, e.g. a `dNSName` in a SAN).
pub const fn context_primitive(n: u8) -> u8 {
    0x80 | n
}

/// Tag, length and content.
pub fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 6);
    out.push(tag);
    let len = content.len();
    if len < 0x80 {
        out.push(u8::try_from(len).unwrap_or(0));
    } else {
        let bytes = len.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | u8::try_from(bytes.len() - skip).unwrap_or(0));
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(content);
    out
}

/// A constructed value of the concatenated `parts`.
pub fn constructed(tag: u8, parts: &[&[u8]]) -> Vec<u8> {
    tlv(tag, &parts.concat())
}

pub fn sequence(parts: &[&[u8]]) -> Vec<u8> {
    constructed(SEQUENCE, parts)
}

/// A non-negative INTEGER from big-endian bytes: leading zeros removed, a
/// zero added when the high bit is set.
pub fn unsigned(bytes: &[u8]) -> Vec<u8> {
    let start = bytes.iter().take_while(|b| **b == 0).count();
    let trimmed = &bytes[start..];
    let mut content = Vec::with_capacity(trimmed.len() + 1);
    if trimmed.first().is_none_or(|b| b & 0x80 != 0) {
        content.push(0);
    }
    content.extend_from_slice(trimmed);
    tlv(INTEGER, &content)
}

/// A BIT STRING of whole bytes.
pub fn bit_string(bytes: &[u8]) -> Vec<u8> {
    let mut content = Vec::with_capacity(bytes.len() + 1);
    content.push(0);
    content.extend_from_slice(bytes);
    tlv(BIT_STRING, &content)
}

pub fn boolean(value: bool) -> Vec<u8> {
    tlv(BOOLEAN, &[if value { 0xff } else { 0 }])
}

/// An OBJECT IDENTIFIER from its arcs (at least two).
pub fn oid(arcs: &[u64]) -> Vec<u8> {
    let mut content = Vec::new();
    let mut push = |mut arc: u64| {
        let mut chunk = vec![u8::try_from(arc & 0x7f).unwrap_or(0)];
        arc >>= 7;
        while arc > 0 {
            chunk.push(0x80 | u8::try_from(arc & 0x7f).unwrap_or(0));
            arc >>= 7;
        }
        chunk.reverse();
        content.extend_from_slice(&chunk);
    };
    push(arcs[0] * 40 + arcs[1]);
    for &arc in &arcs[2..] {
        push(arc);
    }
    tlv(OID, &content)
}

/// UTCTime up to 2049, GeneralizedTime from 2050 on (RFC 5280 § 4.1.2.5),
/// to the second.
pub fn time(at: Timestamp) -> Vec<u8> {
    let civil = TimeZone::UTC.to_datetime(at);
    if civil.year() < 2050 {
        tlv(
            UTC_TIME,
            civil.strftime("%y%m%d%H%M%SZ").to_string().as_bytes(),
        )
    } else {
        tlv(
            GENERALIZED_TIME,
            civil.strftime("%Y%m%d%H%M%SZ").to_string().as_bytes(),
        )
    }
}

/// Reads a time written as UTCTime or GeneralizedTime (UTC, whole seconds).
pub fn parse_time(tag: u8, content: &[u8]) -> anyhow::Result<Timestamp> {
    // Digits and a final `Z` only, so that the fixed slices below never fall
    // inside a multi-byte character.
    let (digits, zone) = content.split_at(content.len().saturating_sub(1));
    ensure!(
        zone == b"Z" && digits.iter().all(u8::is_ascii_digit),
        "unsupported time format"
    );
    let text = std::str::from_utf8(content)?;
    let (year, rest) = match (tag, text.len()) {
        (UTC_TIME, 13) => {
            let yy: i16 = text[..2].parse()?;
            (if yy < 50 { 2000 + yy } else { 1900 + yy }, &text[2..])
        }
        (GENERALIZED_TIME, 15) => (text[..4].parse()?, &text[4..]),
        _ => bail!("unsupported time format"),
    };
    ensure!(rest.ends_with('Z'), "time is not in UTC");
    let num = |range: std::ops::Range<usize>| -> anyhow::Result<i8> { Ok(rest[range].parse()?) };
    let civil = DateTime::new(
        year,
        num(0..2)?,
        num(2..4)?,
        num(4..6)?,
        num(6..8)?,
        num(8..10)?,
        0,
    )?;
    Ok(TimeZone::UTC.to_timestamp(civil)?)
}

/// One element read: its tag, its content and the whole encoding.
#[derive(Debug, Clone, Copy)]
pub struct Element<'a> {
    pub tag: u8,
    pub content: &'a [u8],
    pub raw: &'a [u8],
}

impl<'a> Element<'a> {
    /// A reader over the content of a constructed element.
    pub fn reader(&self) -> Reader<'a> {
        Reader::new(self.content)
    }
}

/// Reads elements one after another.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The tag of the next element without reading it.
    pub fn peek_tag(&self) -> Option<u8> {
        self.data.first().copied()
    }

    /// The next element, of any tag.
    pub fn read(&mut self) -> anyhow::Result<Element<'a>> {
        let data = self.data;
        let (&tag, rest) = data
            .split_first()
            .ok_or_else(|| anyhow!("DER: end of data"))?;
        ensure!(tag & 0x1f != 0x1f, "DER: multi-byte tags are not supported");
        let (&first, rest) = rest
            .split_first()
            .ok_or_else(|| anyhow!("DER: no length"))?;
        let (len, rest) = if first < 0x80 {
            (usize::from(first), rest)
        } else {
            let count = usize::from(first & 0x7f);
            ensure!(
                (1..=4).contains(&count) && rest.len() >= count,
                "DER: bad length"
            );
            let (bytes, rest) = rest.split_at(count);
            ensure!(bytes[0] != 0, "DER: length not in its shortest form");
            let len = bytes
                .iter()
                .fold(0usize, |len, b| (len << 8) | usize::from(*b));
            ensure!(len >= 0x80, "DER: length not in its shortest form");
            (len, rest)
        };
        ensure!(rest.len() >= len, "DER: length beyond the data");
        let header = data.len() - rest.len();
        let raw = &data[..header + len];
        self.data = &data[header + len..];
        Ok(Element {
            tag,
            content: &rest[..len],
            raw,
        })
    }

    /// The next element, which must have `tag`.
    pub fn expect(&mut self, tag: u8) -> anyhow::Result<Element<'a>> {
        let element = self.read()?;
        ensure!(
            element.tag == tag,
            "DER: expected tag {tag:#04x}, found {:#04x}",
            element.tag
        );
        Ok(element)
    }

    /// The next element if it has `tag`.
    pub fn optional(&mut self, tag: u8) -> anyhow::Result<Option<Element<'a>>> {
        if self.peek_tag() == Some(tag) {
            self.read().map(Some)
        } else {
            Ok(None)
        }
    }

    /// Fails if anything is left.
    pub fn finish(&self) -> anyhow::Result<()> {
        ensure!(self.data.is_empty(), "DER: trailing data");
        Ok(())
    }
}

/// The content of a BIT STRING without unused bits.
pub fn bit_string_bytes(content: &[u8]) -> anyhow::Result<&[u8]> {
    match content.split_first() {
        Some((0, bytes)) => Ok(bytes),
        _ => bail!("DER: BIT STRING with unused bits"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_short_and_long() {
        assert_eq!(tlv(OCTET_STRING, &[]), [0x04, 0x00]);
        assert_eq!(tlv(OCTET_STRING, &[7; 0x7f])[..2], [0x04, 0x7f]);
        assert_eq!(tlv(OCTET_STRING, &[7; 0x80])[..3], [0x04, 0x81, 0x80]);
        assert_eq!(
            tlv(OCTET_STRING, &[7; 0x1234])[..4],
            [0x04, 0x82, 0x12, 0x34]
        );
        for len in [0, 1, 0x7f, 0x80, 0xff, 0x100, 0x1234, 70_000] {
            let encoded = tlv(SEQUENCE, &vec![1; len]);
            let mut reader = Reader::new(&encoded);
            let element = reader.expect(SEQUENCE).unwrap();
            assert_eq!(element.content.len(), len);
            assert_eq!(element.raw, encoded);
            reader.finish().unwrap();
        }
    }

    #[test]
    fn rejects_non_canonical_lengths() {
        for bad in [
            &[0x30, 0x81, 0x05, 0, 0, 0, 0, 0][..],
            &[0x30, 0x82, 0x00, 0x90],
            &[0x30, 0x80],
            &[0x30, 0x05, 0, 0],
            &[0x30],
            &[0x1f, 0x01, 0x00],
        ] {
            assert!(Reader::new(bad).read().is_err(), "{bad:02x?}");
        }
    }

    #[test]
    fn integers_are_positive_and_minimal() {
        assert_eq!(unsigned(&[0, 0, 1]), [0x02, 0x01, 0x01]);
        assert_eq!(unsigned(&[0x80]), [0x02, 0x02, 0x00, 0x80]);
        assert_eq!(unsigned(&[]), [0x02, 0x01, 0x00]);
        assert_eq!(unsigned(&[0, 0]), [0x02, 0x01, 0x00]);
    }

    #[test]
    fn object_identifiers() {
        // RFC 8737 id-pe-acmeIdentifier and ecdsa-with-SHA256 as in X.690
        // examples of their registries.
        assert_eq!(
            oid(&[1, 3, 6, 1, 5, 5, 7, 1, 31]),
            [0x06, 0x08, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x01, 0x1f]
        );
        assert_eq!(
            oid(&[1, 2, 840, 10045, 4, 3, 2]),
            [0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02]
        );
    }

    #[test]
    fn times_round_trip() {
        for (at, tag, text) in [
            ("2026-11-27T10:20:30Z", UTC_TIME, "261127102030Z"),
            ("1999-01-02T03:04:05Z", UTC_TIME, "990102030405Z"),
            ("2050-01-01T00:00:00Z", GENERALIZED_TIME, "20500101000000Z"),
        ] {
            let at: Timestamp = at.parse().unwrap();
            let encoded = time(at);
            let element = Reader::new(&encoded).read().unwrap();
            assert_eq!(element.tag, tag);
            assert_eq!(element.content, text.as_bytes());
            assert_eq!(parse_time(element.tag, element.content).unwrap(), at);
        }
        assert!(parse_time(UTC_TIME, b"2611271020Z").is_err());
        assert!(parse_time(UTC_TIME, b"261127102030+").is_err());
        // Multi-byte characters of the right length: an error, no panic.
        assert!(parse_time(UTC_TIME, "2ü1127102030".as_bytes()).is_err());
        assert!(parse_time(GENERALIZED_TIME, "20ü61127102030".as_bytes()).is_err());
        assert!(parse_time(UTC_TIME, b"").is_err());
    }
}
