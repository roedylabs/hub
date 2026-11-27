//! An in-process authoritative name server for the DNS-01 tests: UDP and
//! TCP on the same loopback port, TXT records only. Answers carry `AA`,
//! the owner as a compression pointer to the question, and a record's
//! strings as given; unknown names get `NXDOMAIN`. A knob truncates UDP
//! answers (`TC`) so that the client has to ask again over TCP.

#![allow(dead_code, reason = "each test uses its own part")]

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Instant,
};

use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, UdpSocket},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Udp,
    Tcp,
}

#[derive(Default)]
struct Inner {
    /// TXT records by lower-case name; each record a list of strings.
    records: HashMap<String, Vec<Vec<String>>>,
    /// When each name was last set.
    set_at: HashMap<String, Instant>,
    truncate_udp: bool,
    queries: Vec<(String, Transport)>,
}

#[derive(Clone)]
pub struct FakeDns {
    pub addr: SocketAddr,
    inner: Arc<Mutex<Inner>>,
}

impl FakeDns {
    pub async fn start() -> Self {
        // UDP and TCP on the same port: take UDP's and try TCP there.
        let (udp, tcp) = loop {
            let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            if let Ok(tcp) = TcpListener::bind(udp.local_addr().unwrap()).await {
                break (udp, tcp);
            }
        };
        let fake = Self {
            addr: udp.local_addr().unwrap(),
            inner: Arc::default(),
        };
        let server = fake.clone();
        tokio::spawn(async move {
            let mut buffer = vec![0; 512];
            loop {
                let Ok((length, peer)) = udp.recv_from(&mut buffer).await else {
                    return;
                };
                if let Some(answer) = server.answer(&buffer[..length], Transport::Udp) {
                    let _ = udp.send_to(&answer, peer).await;
                }
            }
        });
        let server = fake.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = tcp.accept().await else {
                    return;
                };
                let server = server.clone();
                tokio::spawn(async move {
                    let Ok(length) = stream.read_u16().await else {
                        return;
                    };
                    let mut query = vec![0; usize::from(length)];
                    if stream.read_exact(&mut query).await.is_err() {
                        return;
                    }
                    if let Some(answer) = server.answer(&query, Transport::Tcp) {
                        let mut framed =
                            u16::try_from(answer.len()).unwrap().to_be_bytes().to_vec();
                        framed.extend_from_slice(&answer);
                        let _ = stream.write_all(&framed).await;
                    }
                });
            }
        });
        fake
    }

    /// Sets the TXT records of `name`, one string each; none removes them.
    pub fn set(&self, name: &str, values: &[String]) {
        self.set_strings(
            name,
            values.iter().map(|value| vec![value.clone()]).collect(),
        );
    }

    pub fn set_strings(&self, name: &str, records: Vec<Vec<String>>) {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        let mut inner = self.lock();
        inner.set_at.insert(name.clone(), Instant::now());
        if records.is_empty() {
            inner.records.remove(&name);
        } else {
            inner.records.insert(name, records);
        }
    }

    /// The records of `name`, each record's strings joined.
    pub fn txt(&self, name: &str) -> Vec<String> {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        self.lock()
            .records
            .get(&name)
            .map(|records| records.iter().map(|strings| strings.concat()).collect())
            .unwrap_or_default()
    }

    pub fn set_at(&self, name: &str) -> Option<Instant> {
        self.lock().set_at.get(&name.to_ascii_lowercase()).copied()
    }

    pub fn truncate_udp(&self, on: bool) {
        self.lock().truncate_udp = on;
    }

    pub fn queries(&self) -> Vec<(String, Transport)> {
        self.lock().queries.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    /// The answer to a query; `None` for anything that is not one.
    fn answer(&self, query: &[u8], transport: Transport) -> Option<Vec<u8>> {
        if query.len() < 12 || query[2] & 0x80 != 0 || query[4..6] != [0, 1] {
            return None;
        }
        // The question: uncompressed labels, type, class.
        let mut pos = 12;
        let mut labels = Vec::new();
        loop {
            let length = usize::from(*query.get(pos)?);
            pos += 1;
            if length == 0 {
                break;
            }
            labels.push(String::from_utf8_lossy(query.get(pos..pos + length)?).to_lowercase());
            pos += length;
        }
        let question = query.get(12..pos + 4)?;
        let kind = u16::from_be_bytes([query[pos], query[pos + 1]]);
        let name = labels.join(".");

        let mut inner = self.lock();
        inner.queries.push((name.clone(), transport));
        let records = inner.records.get(&name).cloned();
        let truncate = transport == Transport::Udp && inner.truncate_udp;
        drop(inner);

        let answers: Vec<Vec<String>> = match (&records, kind) {
            (Some(records), 16) => records.clone(),
            _ => Vec::new(),
        };
        // QR, AA (and TC); NXDOMAIN for unknown names.
        let mut flags: u16 = 0x8400;
        if records.is_none() {
            flags |= 3;
        }
        let mut answer = query[..2].to_vec();
        if truncate {
            flags |= 0x0200;
            answer.extend_from_slice(&flags.to_be_bytes());
            answer.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]);
            answer.extend_from_slice(question);
            return Some(answer);
        }
        answer.extend_from_slice(&flags.to_be_bytes());
        answer.extend_from_slice(&[0, 1]);
        answer.extend_from_slice(&u16::try_from(answers.len()).unwrap().to_be_bytes());
        answer.extend_from_slice(&[0, 0, 0, 0]);
        answer.extend_from_slice(question);
        for strings in answers {
            let mut data = Vec::new();
            for string in strings {
                data.push(u8::try_from(string.len()).unwrap());
                data.extend_from_slice(string.as_bytes());
            }
            // The owner: a pointer to the question's name.
            answer.extend_from_slice(&[0xc0, 12, 0, 16, 0, 1, 0, 0, 0, 60]);
            answer.extend_from_slice(&u16::try_from(data.len()).unwrap().to_be_bytes());
            answer.extend_from_slice(&data);
        }
        Some(answer)
    }
}
