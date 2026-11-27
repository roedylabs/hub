//! The HTTPS listener for `axum::serve`: a task accepts TCP connections and
//! runs each handshake in its own task (a slow client holds up no other),
//! with a deadline and at most [`MAX_PENDING`] at a time. Finished
//! connections go to axum; those of a TLS-ALPN-01 validation (`acme-tls/1`)
//! are closed right after the handshake, which is all the validation needs
//! (RFC 8737 § 3).

use std::{io, net::SocketAddr, sync::Arc, time::Duration};

use rustls::ServerConfig;
use tokio::{
    io::AsyncWriteExt as _,
    net::{TcpListener, TcpStream},
    sync::{Semaphore, mpsc},
    task::{JoinHandle, JoinSet},
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};

use super::ACME_TLS_ALPN;

/// Longest time for a TLS handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Finished handshakes waiting for axum.
const QUEUE: usize = 64;
/// Connections in a handshake or waiting for axum, at most.
const MAX_PENDING: usize = 512;

pub struct TlsListener {
    connections: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local_addr: SocketAddr,
    accept: JoinHandle<()>,
}

impl TlsListener {
    pub fn new(listener: TcpListener, config: Arc<ServerConfig>) -> io::Result<Self> {
        let local_addr = listener.local_addr()?;
        let (sender, connections) = mpsc::channel(QUEUE);
        let acceptor = TlsAcceptor::from(config);
        let accept = tokio::spawn(accept_loop(listener, acceptor, sender));
        Ok(Self {
            connections,
            local_addr,
            accept,
        })
    }
}

impl Drop for TlsListener {
    fn drop(&mut self) {
        // Stops accepting: axum dropped the listener at shutdown.
        self.accept.abort();
    }
}

async fn accept_loop(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    sender: mpsc::Sender<(TlsStream<TcpStream>, SocketAddr)>,
) {
    // Owned here: aborting this task (the listener dropped) aborts every
    // handshake still running.
    let mut handshakes = JoinSet::new();
    let pending = Arc::new(Semaphore::new(MAX_PENDING));
    loop {
        // Reap finished handshakes so the set does not grow.
        while handshakes.try_join_next().is_some() {}
        // At most MAX_PENDING connections between accept and axum; beyond
        // that the kernel's backlog holds new ones.
        let Ok(permit) = pending.clone().acquire_owned().await else {
            return;
        };
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                // As axum does for its own listener: e.g. too many open
                // files; wait instead of spinning.
                tracing::debug!("TLS listener: accept failed: {error}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        if sender.is_closed() {
            return;
        }
        let acceptor = acceptor.clone();
        let sender = sender.clone();
        handshakes.spawn(async move {
            // Held until the connection is queued for axum or closed.
            let _permit = permit;
            let _ = stream.set_nodelay(true);
            let handshake = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await;
            let mut tls = match handshake {
                Ok(Ok(tls)) => tls,
                Ok(Err(error)) => {
                    tracing::debug!(%peer, "TLS handshake failed: {error}");
                    return;
                }
                Err(_) => {
                    tracing::debug!(%peer, "TLS handshake timed out");
                    return;
                }
            };
            if tls.get_ref().1.alpn_protocol() == Some(ACME_TLS_ALPN) {
                tracing::info!(%peer, "TLS-ALPN-01 validation connected");
                let _ = tls.shutdown().await;
                return;
            }
            let _ = sender.send((tls, peer)).await;
        });
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.connections.recv().await {
            Some(connection) => connection,
            // The accept task never ends on its own; wait for shutdown.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local_addr)
    }
}
