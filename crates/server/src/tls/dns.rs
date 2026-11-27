//! A minimal DNS client (RFC 1035) for one question: which TXT records an
//! authoritative name server serves for one name. The DNS-01 challenge
//! ([`super::dns01`]) asks the zone's name servers until the new record is
//! there, before the CA is told to validate.
//!
//! Hand-written instead of a resolver library (decided on 2026-11-27): one
//! query type, no recursion, no cache – a query is ~30 bytes and the answer
//! needs only the header, the question and the answer records. UDP first,
//! TCP when the answer comes back truncated (§ 4.2.2: a two-byte length
//! before each message). No EDNS: a TXT record of the challenge fits into
//! 512 bytes, and a larger answer goes over TCP.

use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use anyhow::{Context as _, anyhow, bail};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpStream, UdpSocket},
};

/// `TXT` (RFC 1035 § 3.2.2).
pub const TYPE_TXT: u16 = 16;
/// `IN` (§ 3.2.4).
pub const CLASS_IN: u16 = 1;
/// The header's size (§ 4.1.1).
const HEADER: usize = 12;
/// Longest name in wire format (§ 2.3.4).
const MAX_NAME: usize = 255;
/// Longest label (§ 2.3.4).
const MAX_LABEL: usize = 63;
/// Largest UDP answer read: without EDNS a server sends at most 512 bytes.
const UDP_BUFFER: usize = 4096;

/// Response codes the propagation check tells apart (§ 4.1.1).
pub const RCODE_NOERROR: u8 = 0;
pub const RCODE_NXDOMAIN: u8 = 3;

/// What a name server answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub id: u16,
    /// `AA`: the server is authoritative for the name.
    pub authoritative: bool,
    /// `TC`: the answer did not fit (UDP); ask again over TCP.
    pub truncated: bool,
    pub rcode: u8,
    /// The TXT records of the name asked for, each record's strings joined
    /// (as the CA reads them: RFC 8555 § 8.4 takes the record's value).
    pub txt: Vec<String>,
}

/// A query for the TXT records of `name`, without recursion (`RD` = 0): the
/// question goes to the authoritative servers themselves.
pub fn query(id: u16, name: &str) -> anyhow::Result<Vec<u8>> {
    let mut message = Vec::with_capacity(HEADER + name.len() + 6);
    message.extend_from_slice(&id.to_be_bytes());
    // QR = 0 (query), OPCODE = 0 (standard), AA, TC, RD = 0; RA, Z, RCODE = 0.
    message.extend_from_slice(&[0, 0]);
    // One question, no answer, authority or additional records.
    message.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]);
    encode_name(&mut message, name)?;
    message.extend_from_slice(&TYPE_TXT.to_be_bytes());
    message.extend_from_slice(&CLASS_IN.to_be_bytes());
    Ok(message)
}

/// Appends `name` in wire format: length-prefixed labels and the root.
fn encode_name(out: &mut Vec<u8>, name: &str) -> anyhow::Result<()> {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty() {
        bail!("empty DNS name");
    }
    let start = out.len();
    for label in name.split('.') {
        let length = u8::try_from(label.len())
            .ok()
            .filter(|length| (1..=MAX_LABEL as u8).contains(length))
            .ok_or_else(|| anyhow!("invalid DNS label in {name}"))?;
        if !label.is_ascii() {
            bail!("DNS name {name} is not ASCII");
        }
        out.push(length);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    if out.len() - start > MAX_NAME {
        bail!("DNS name {name} is too long");
    }
    Ok(())
}

/// Reads the answer to [`query`] with `id` for `name`. A truncated answer
/// is returned with `truncated` set and its records left unread – a
/// truncated UDP message may end in the middle of one.
pub fn parse_response(message: &[u8], id: u16, name: &str) -> anyhow::Result<Response> {
    let mut reader = Reader { message, pos: 0 };
    let got_id = reader.u16()?;
    let flags = reader.u16()?;
    let questions = reader.u16()?;
    let answers = reader.u16()?;
    // Authority and additional records are not needed.
    reader.u16()?;
    reader.u16()?;
    if got_id != id {
        bail!("DNS answer with another ID");
    }
    if flags & 0x8000 == 0 {
        bail!("DNS message is not an answer");
    }
    if (flags >> 11) & 0x0f != 0 {
        bail!("DNS answer with another opcode");
    }
    let mut response = Response {
        id,
        authoritative: flags & 0x0400 != 0,
        truncated: flags & 0x0200 != 0,
        rcode: (flags & 0x000f) as u8,
        txt: Vec::new(),
    };
    if response.truncated {
        return Ok(response);
    }
    if questions != 1 {
        bail!("DNS answer with {questions} questions");
    }
    let asked = reader.name()?;
    let (kind, class) = (reader.u16()?, reader.u16()?);
    if !same_name(&asked, name) || kind != TYPE_TXT || class != CLASS_IN {
        bail!("DNS answer to another question");
    }
    for _ in 0..answers {
        let owner = reader.name()?;
        let (kind, class) = (reader.u16()?, reader.u16()?);
        // TTL
        reader.take(4)?;
        let length = usize::from(reader.u16()?);
        let data = reader.take(length)?;
        // Other records (a CNAME, signatures) are skipped: the hub sets the
        // TXT record at the name itself.
        if kind == TYPE_TXT && class == CLASS_IN && same_name(&owner, name) {
            response.txt.push(txt_value(data)?);
        }
    }
    Ok(response)
}

/// A TXT record's value: its character strings (§ 3.3.14) joined.
fn txt_value(mut data: &[u8]) -> anyhow::Result<String> {
    let mut value = Vec::with_capacity(data.len());
    while let Some((&length, rest)) = data.split_first() {
        let length = usize::from(length);
        if rest.len() < length {
            bail!("TXT record cut off");
        }
        value.extend_from_slice(&rest[..length]);
        data = &rest[length..];
    }
    Ok(String::from_utf8_lossy(&value).into_owned())
}

/// Names compare without case and without the root's dot (§ 2.3.3).
fn same_name(a: &str, b: &str) -> bool {
    a.trim_end_matches('.')
        .eq_ignore_ascii_case(b.trim_end_matches('.'))
}

struct Reader<'a> {
    message: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> anyhow::Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.message.len())
            .ok_or_else(|| anyhow!("DNS message cut off"))?;
        let bytes = &self.message[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn u16(&mut self) -> anyhow::Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// A name, following compression pointers (§ 4.1.4); dotted, without
    /// the root's dot. Pointers must point backwards, which also rules out
    /// loops.
    fn name(&mut self) -> anyhow::Result<String> {
        let mut name = String::new();
        let mut wire_length = 1;
        let mut pos = self.pos;
        // Where reading goes on after the name: after its first pointer, or
        // after its end.
        let mut resume = None;
        loop {
            let &length = self
                .message
                .get(pos)
                .ok_or_else(|| anyhow!("DNS name cut off"))?;
            match length & 0xc0 {
                0x00 if length == 0 => {
                    pos += 1;
                    break;
                }
                0x00 => {
                    let length = usize::from(length);
                    let label = self
                        .message
                        .get(pos + 1..pos + 1 + length)
                        .ok_or_else(|| anyhow!("DNS name cut off"))?;
                    wire_length += length + 1;
                    if wire_length > MAX_NAME {
                        bail!("DNS name too long");
                    }
                    if !name.is_empty() {
                        name.push('.');
                    }
                    name.push_str(&String::from_utf8_lossy(label));
                    pos += 1 + length;
                }
                0xc0 => {
                    let &low = self
                        .message
                        .get(pos + 1)
                        .ok_or_else(|| anyhow!("DNS name cut off"))?;
                    let target = usize::from(length & 0x3f) << 8 | usize::from(low);
                    if target >= pos {
                        bail!("DNS name with a forward pointer");
                    }
                    resume.get_or_insert(pos + 2);
                    pos = target;
                }
                // 0x40 and 0x80: extended label types (RFC 6891 § 5), unused.
                _ => bail!("DNS name with an unknown label type"),
            }
        }
        self.pos = resume.unwrap_or(pos);
        Ok(name)
    }
}

/// Asks `server` for the TXT records of `name`: over UDP, again over TCP if
/// the answer is truncated. Both together must finish within `timeout`.
pub async fn lookup_txt(
    server: SocketAddr,
    name: &str,
    timeout: Duration,
) -> anyhow::Result<Response> {
    let deadline = tokio::time::Instant::now() + timeout;
    let id = u16::from_be_bytes(crate::random::bytes::<2>()?);
    let query = query(id, name)?;
    let response = tokio::time::timeout_at(deadline, udp(server, &query, id, name))
        .await
        .map_err(|_| anyhow!("no DNS answer from {server} within {timeout:?}"))??;
    if !response.truncated {
        return Ok(response);
    }
    tokio::time::timeout_at(deadline, tcp(server, &query, id, name))
        .await
        .map_err(|_| anyhow!("no DNS answer from {server} over TCP within {timeout:?}"))?
}

async fn udp(server: SocketAddr, query: &[u8], id: u16, name: &str) -> anyhow::Result<Response> {
    let local = if server.is_ipv4() {
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))
    } else {
        SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))
    };
    let socket = UdpSocket::bind(local).await.context("DNS over UDP")?;
    // Connected: the socket only receives from the server.
    socket
        .connect(server)
        .await
        .with_context(|| format!("DNS over UDP to {server}"))?;
    socket
        .send(query)
        .await
        .with_context(|| format!("DNS over UDP to {server}"))?;
    let mut buffer = vec![0; UDP_BUFFER];
    loop {
        let length = socket
            .recv(&mut buffer)
            .await
            .with_context(|| format!("DNS over UDP from {server}"))?;
        // A late answer to an earlier query (another ID) is not this one's;
        // wait on until the timeout.
        let message = &buffer[..length];
        if message.len() >= 2 && message[..2] != id.to_be_bytes() {
            continue;
        }
        return parse_response(message, id, name);
    }
}

async fn tcp(server: SocketAddr, query: &[u8], id: u16, name: &str) -> anyhow::Result<Response> {
    let mut stream = TcpStream::connect(server)
        .await
        .with_context(|| format!("DNS over TCP to {server}"))?;
    let length = u16::try_from(query.len()).context("DNS query too long")?;
    let mut framed = Vec::with_capacity(query.len() + 2);
    framed.extend_from_slice(&length.to_be_bytes());
    framed.extend_from_slice(query);
    stream
        .write_all(&framed)
        .await
        .with_context(|| format!("DNS over TCP to {server}"))?;
    let length = stream
        .read_u16()
        .await
        .with_context(|| format!("DNS over TCP from {server}"))?;
    let mut message = vec![0; usize::from(length)];
    stream
        .read_exact(&mut message)
        .await
        .with_context(|| format!("DNS over TCP from {server}"))?;
    let response = parse_response(&message, id, name)?;
    if response.truncated {
        bail!("DNS answer from {server} truncated over TCP");
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAME: &str = "_acme-challenge.hub.example.test";

    /// A response header: ID, flags, counts.
    fn header(id: u16, flags: u16, questions: u16, answers: u16) -> Vec<u8> {
        let mut message = Vec::new();
        for value in [id, flags, questions, answers, 0, 0] {
            message.extend_from_slice(&value.to_be_bytes());
        }
        message
    }

    /// The question of `NAME` as the server echoes it, uncompressed.
    fn question(message: &mut Vec<u8>) {
        encode_name(message, NAME).unwrap();
        message.extend_from_slice(&TYPE_TXT.to_be_bytes());
        message.extend_from_slice(&CLASS_IN.to_be_bytes());
    }

    /// A record with `owner` already in wire format.
    fn record(message: &mut Vec<u8>, owner: &[u8], kind: u16, data: &[u8]) {
        message.extend_from_slice(owner);
        message.extend_from_slice(&kind.to_be_bytes());
        message.extend_from_slice(&CLASS_IN.to_be_bytes());
        message.extend_from_slice(&300u32.to_be_bytes());
        message.extend_from_slice(&u16::try_from(data.len()).unwrap().to_be_bytes());
        message.extend_from_slice(data);
    }

    fn strings(parts: &[&str]) -> Vec<u8> {
        let mut data = Vec::new();
        for part in parts {
            data.push(u8::try_from(part.len()).unwrap());
            data.extend_from_slice(part.as_bytes());
        }
        data
    }

    #[test]
    fn builds_a_txt_query_without_recursion() {
        let query = query(0xbeef, "_acme-challenge.Hub.example.test.").unwrap();
        let mut expected = vec![0xbe, 0xef, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        expected.push(15);
        expected.extend_from_slice(b"_acme-challenge");
        expected.push(3);
        expected.extend_from_slice(b"Hub");
        expected.push(7);
        expected.extend_from_slice(b"example");
        expected.push(4);
        expected.extend_from_slice(b"test");
        expected.extend_from_slice(&[0, 0, 16, 0, 1]);
        assert_eq!(query, expected);
    }

    #[test]
    fn rejects_names_that_do_not_fit() {
        assert!(query(1, "").is_err());
        assert!(query(1, "a..b").is_err());
        assert!(query(1, &format!("{}.test", "a".repeat(64))).is_err());
        let long = vec!["a".repeat(63); 4].join(".");
        assert!(query(1, &long).is_err(), "256 bytes in wire format");
        assert!(query(1, "bücher.test").is_err());
    }

    #[test]
    fn reads_txt_records_with_compression_and_several_strings() {
        // AA set, two TXT records and a signature that is skipped.
        let mut message = header(7, 0x8400, 1, 3);
        question(&mut message);
        // The owner as a pointer to the question's name at offset 12.
        let pointer = [0xc0, 12];
        record(&mut message, &pointer, TYPE_TXT, &strings(&["abc", "def"]));
        record(&mut message, &pointer, 46, &[1, 2, 3]);
        // The owner as "_acme-challenge" + a pointer to "hub.example.test"
        // (offset 12 + 16).
        let mut owner = vec![15];
        owner.extend_from_slice(b"_ACME-challenge");
        owner.extend_from_slice(&[0xc0, 28]);
        record(&mut message, &owner, TYPE_TXT, &strings(&["xyz"]));
        let response = parse_response(&message, 7, NAME).unwrap();
        assert_eq!(
            response,
            Response {
                id: 7,
                authoritative: true,
                truncated: false,
                rcode: RCODE_NOERROR,
                txt: vec!["abcdef".into(), "xyz".into()],
            }
        );
    }

    #[test]
    fn skips_records_of_other_names() {
        let mut message = header(7, 0x8400, 1, 2);
        question(&mut message);
        let mut other = Vec::new();
        encode_name(&mut other, "other.example.test").unwrap();
        record(&mut message, &other, TYPE_TXT, &strings(&["no"]));
        record(&mut message, &[0xc0, 12], TYPE_TXT, &strings(&["yes"]));
        let response = parse_response(&message, 7, NAME).unwrap();
        assert_eq!(response.txt, ["yes"]);
    }

    #[test]
    fn reads_nxdomain_and_truncation() {
        let mut message = header(9, 0x8403, 1, 0);
        question(&mut message);
        let response = parse_response(&message, 9, NAME).unwrap();
        assert_eq!(response.rcode, RCODE_NXDOMAIN);
        assert!(response.txt.is_empty());

        // Truncated: the rest is not read, even if it is cut off.
        let mut message = header(9, 0x8600, 1, 5);
        message.extend_from_slice(&[15, b'_']);
        let response = parse_response(&message, 9, NAME).unwrap();
        assert!(response.truncated);
        assert!(response.txt.is_empty());
    }

    #[test]
    fn rejects_broken_or_foreign_answers() {
        let mut good = header(7, 0x8400, 1, 1);
        question(&mut good);
        record(&mut good, &[0xc0, 12], TYPE_TXT, &strings(&["v"]));
        parse_response(&good, 7, NAME).unwrap();

        // Another ID, a query, another opcode, another question.
        assert!(parse_response(&good, 8, NAME).is_err());
        let mut query = good.clone();
        query[2] = 0x04;
        assert!(parse_response(&query, 7, NAME).is_err());
        let mut opcode = good.clone();
        opcode[2] = 0x84 | 0x10;
        assert!(parse_response(&opcode, 7, NAME).is_err());
        assert!(parse_response(&good, 7, "other.example.test").is_err());

        // Cut off anywhere.
        for end in 0..good.len() {
            assert!(parse_response(&good[..end], 7, NAME).is_err(), "{end}");
        }
        // A TXT string longer than its record.
        let mut long = header(7, 0x8400, 1, 1);
        question(&mut long);
        record(&mut long, &[0xc0, 12], TYPE_TXT, &[5, b'a']);
        assert!(parse_response(&long, 7, NAME).is_err());
    }

    #[test]
    fn rejects_pointer_loops_and_forward_pointers() {
        // The answer's owner points at itself.
        let mut message = header(7, 0x8400, 1, 1);
        question(&mut message);
        let at = u8::try_from(message.len()).unwrap();
        record(&mut message, &[0xc0, at], TYPE_TXT, &strings(&["v"]));
        assert!(parse_response(&message, 7, NAME).is_err());

        // Two pointers pointing at each other.
        let mut message = header(7, 0x8400, 1, 1);
        question(&mut message);
        let at = u8::try_from(message.len()).unwrap();
        record(&mut message, &[0xc0, at + 2, 0xc0, at], TYPE_TXT, &[]);
        assert!(parse_response(&message, 7, NAME).is_err());

        // Extended label types.
        let mut message = header(7, 0x8400, 1, 1);
        question(&mut message);
        record(&mut message, &[0x40, 0], TYPE_TXT, &[]);
        assert!(parse_response(&message, 7, NAME).is_err());
    }

    #[test]
    fn joins_empty_and_long_strings() {
        assert_eq!(txt_value(&[]).unwrap(), "");
        assert_eq!(txt_value(&[0, 1, b'a', 0]).unwrap(), "a");
        let long = "x".repeat(255);
        let mut data = strings(&[&long]);
        data.extend_from_slice(&strings(&["y"]));
        assert_eq!(txt_value(&data).unwrap(), format!("{long}y"));
    }
}
