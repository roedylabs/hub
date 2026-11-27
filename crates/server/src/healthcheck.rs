//! `roedy-hub healthcheck`: queries `/healthz` of the running server's public
//! listener. The image contains no curl, so the command speaks HTTP/1.1
//! directly over TCP – with built-in TLS over TLS ([`check_tls`]), with the
//! public host as SNI and without checking the certificate: it is the hub's
//! own, reached over loopback. Before the first certificate exists the check
//! fails, as the hub then serves nothing yet.
//!
//! Taken over from Roedy Mail (`crates/server/src/healthcheck.rs`), plus TLS.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context as _, anyhow, bail};
use rustls::{
    ClientConfig, DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{CryptoProvider, ring::default_provider},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use tokio::{
    io::{
        AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _,
        BufReader,
    },
    net::TcpStream,
};
use tokio_rustls::TlsConnector;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

/// Longest accepted status line.
const MAX_STATUS_LINE: u64 = 1024;

/// Succeeds if `GET /healthz` responds with 200.
pub async fn check(listen: SocketAddr, timeout: Duration) -> anyhow::Result<()> {
    let addrs = probe_addrs(listen);
    let shown = addrs
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" or ");
    tokio::time::timeout(timeout, async {
        // Tries the addresses in order.
        let stream = TcpStream::connect(&addrs[..])
            .await
            .with_context(|| format!("no connection to {shown}"))?;
        let addr = stream.peer_addr()?;
        request(stream, &addr.to_string()).await
    })
    .await
    .map_err(|_| anyhow!("{shown} does not respond within {timeout:?}"))?
}

/// As [`check`], over TLS with `host` as SNI and `Host`.
pub async fn check_tls(listen: SocketAddr, host: &str, timeout: Duration) -> anyhow::Result<()> {
    let addrs = probe_addrs(listen);
    let shown = addrs
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" or ");
    let provider = Arc::new(default_provider());
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(OwnServer(provider)))
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned())
        .map_err(|_| anyhow!("{host} is not a valid server name"))?;
    tokio::time::timeout(timeout, async {
        let stream = TcpStream::connect(&addrs[..])
            .await
            .with_context(|| format!("no connection to {shown}"))?;
        let stream = TlsConnector::from(Arc::new(config))
            .connect(name, stream)
            .await
            .with_context(|| format!("no TLS connection to {shown} (no certificate yet?)"))?;
        request(stream, host).await
    })
    .await
    .map_err(|_| anyhow!("{shown} does not respond within {timeout:?}"))?
}

/// Accepts the server's certificate unchecked – the healthcheck talks to
/// its own process over loopback – but checks the handshake signatures.
#[derive(Debug)]
struct OwnServer(Arc<CryptoProvider>);

impl ServerCertVerifier for OwnServer {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// If the server listens on all addresses, the check runs locally over
/// loopback. On Linux `[::]` also accepts IPv4; containers without IPv6 lack
/// `::1`, so `127.0.0.1` is tried as well.
fn probe_addrs(listen: SocketAddr) -> Vec<SocketAddr> {
    let port = listen.port();
    let v4 = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => vec![v4],
        IpAddr::V6(ip) if ip.is_unspecified() => {
            vec![SocketAddr::from((Ipv6Addr::LOCALHOST, port)), v4]
        }
        _ => vec![listen],
    }
}

async fn request(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    host: &str,
) -> anyhow::Result<()> {
    let addr = host;
    stream
        .write_all(
            format!("GET /healthz HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await?;

    let mut line = String::new();
    BufReader::new(stream.take(MAX_STATUS_LINE))
        .read_line(&mut line)
        .await
        .context("no response")?;
    let line = line.trim_end();
    let mut parts = line.split(' ');
    match (parts.next(), parts.next()) {
        (Some(version), Some("200")) if version.starts_with("HTTP/1.") => Ok(()),
        _ if line.is_empty() => bail!("empty response from {addr}"),
        _ => bail!("/healthz responds with \"{line}\""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// Answers exactly one connection with `response`.
    async fn serve_once(response: &'static str) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await.unwrap();
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        addr
    }

    #[test]
    fn probes_loopback_for_unspecified_address() {
        let probe = |s: &str| {
            probe_addrs(s.parse().unwrap())
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(probe("0.0.0.0:8080"), ["127.0.0.1:8080"]);
        assert_eq!(probe("[::]:8080"), ["[::1]:8080", "127.0.0.1:8080"]);
        assert_eq!(probe("192.0.2.7:9000"), ["192.0.2.7:9000"]);
    }

    #[tokio::test]
    async fn unspecified_v6_falls_back_to_v4() {
        let addr = serve_once("HTTP/1.1 200 OK\r\n\r\n").await;
        let listen = SocketAddr::from((Ipv6Addr::UNSPECIFIED, addr.port()));
        check(listen, DEFAULT_TIMEOUT).await.unwrap();
    }

    #[tokio::test]
    async fn accepts_ok() {
        let addr = serve_once("HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n").await;
        check(addr, DEFAULT_TIMEOUT).await.unwrap();
    }

    #[tokio::test]
    async fn rejects_error_status() {
        let addr = serve_once("HTTP/1.1 503 Service Unavailable\r\n\r\n").await;
        let err = check(addr, DEFAULT_TIMEOUT).await.unwrap_err();
        assert!(err.to_string().contains("503"), "{err}");
    }

    #[tokio::test]
    async fn rejects_garbage() {
        let addr = serve_once("SSH-2.0-OpenSSH\r\n").await;
        assert!(check(addr, DEFAULT_TIMEOUT).await.is_err());
    }

    #[tokio::test]
    async fn fails_without_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let err = check(addr, DEFAULT_TIMEOUT).await.unwrap_err();
        assert!(err.to_string().contains("no connection"), "{err}");
    }

    #[tokio::test]
    async fn times_out_on_silent_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let err = check(addr, Duration::from_millis(100)).await.unwrap_err();
        assert!(err.to_string().contains("does not respond"), "{err}");
        drop(listener);
    }
}
