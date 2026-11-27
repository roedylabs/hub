//! The gateway's client to the apps and the timing of a request (contract
//! § 3.4):
//!
//! - [`client`]: hyper's pooled client over HTTP/1.1, TLS for an `https`
//!   internal address; the connect deadline covers the name lookup, the TCP
//!   connection and the TLS handshake ([`Deadline`]), so that a stalled step
//!   ends in `502 app_unreachable`, never in the response timeout.
//! - [`Upload`]: the request body as it arrives, telling when it is sent
//!   completely – the time until the app's response headers counts only
//!   from then, so that a long upload has no time limit.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use axum::body::{Body, Bytes};
use hyper::{
    Uri,
    body::{Body as HttpBody, Frame, SizeHint},
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use rustls_platform_verifier::ConfigVerifierExt as _;
use tokio::sync::oneshot;

pub type AppClient = Client<Deadline<HttpsConnector<HttpConnector>>, Upload>;

/// hyper's pooled client: HTTP/1.1 only (contract § 3.4), TLS for an
/// `https` internal address with the system's roots, as the health checks.
pub fn client(connect: Duration) -> anyhow::Result<AppClient> {
    // As in `mailer::http_client`: ring for the whole process, if no
    // provider is set yet.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let tls = rustls::ClientConfig::with_platform_verifier()
        .map_err(|error| anyhow::anyhow!("TLS for the apps not set up: {error}"))?;
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    http.set_nodelay(true);
    let connector = HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_or_http()
        .enable_http1()
        .wrap_connector(http);
    Ok(Client::builder(TokioExecutor::new())
        .pool_idle_timeout(Duration::from_secs(60))
        .build(Deadline {
            inner: connector,
            within: connect,
        }))
}

/// A connector whose whole connection – lookup, TCP, TLS – has to be ready
/// `within`; hyper reports its error as a connect error.
#[derive(Debug, Clone)]
pub struct Deadline<C> {
    inner: C,
    within: Duration,
}

/// The connection was not ready in time.
#[derive(Debug)]
pub struct ConnectTimeout(Duration);

impl std::fmt::Display for ConnectTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no connection within {:?}", self.0)
    }
}

impl std::error::Error for ConnectTimeout {}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

impl<C> tower_service::Service<Uri> for Deadline<C>
where
    C: tower_service::Service<Uri> + Send,
    C::Future: Send + 'static,
    C::Error: Into<BoxError>,
{
    type Response = C::Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<C::Response, BoxError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let within = self.within;
        let connecting = self.inner.call(uri);
        Box::pin(async move {
            match tokio::time::timeout(within, connecting).await {
                Ok(connected) => connected.map_err(Into::into),
                Err(_) => Err(Box::new(ConnectTimeout(within)) as BoxError),
            }
        })
    }
}

/// The request body for the app; [`Self::new`] also returns a future that
/// is ready once the body is sent completely (or broke off, or was dropped).
pub struct Upload {
    inner: Body,
    sent: Option<oneshot::Sender<()>>,
}

impl Upload {
    pub fn new(inner: Body) -> (Self, impl Future<Output = ()> + Send + 'static) {
        let (sender, receiver) = oneshot::channel();
        let mut upload = Self {
            inner,
            sent: Some(sender),
        };
        // Without a body (most requests) the timer starts at once.
        if upload.inner.is_end_stream() {
            upload.done();
        }
        (upload, async move {
            // An error means the body is gone: sent as far as it goes.
            let _ = receiver.await;
        })
    }

    fn done(&mut self) {
        if let Some(sent) = self.sent.take() {
            let _ = sent.send(());
        }
    }
}

impl HttpBody for Upload {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.inner).poll_frame(cx);
        match &polled {
            Poll::Ready(None | Some(Err(_))) => this.done(),
            Poll::Ready(Some(Ok(_))) if this.inner.is_end_stream() => this.done(),
            _ => {}
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// Waits for the app's response: without a limit while the body is still
/// being sent, then at most `limit` (contract § 3.4: until the response
/// headers). An early response ends the wait at once. `None` after the
/// limit.
pub async fn respond<F: Future>(
    response: F,
    sent: impl Future<Output = ()>,
    limit: Duration,
) -> Option<F::Output> {
    tokio::pin!(response);
    tokio::select! {
        biased;
        response = &mut response => return Some(response),
        () = sent => {}
    }
    tokio::time::timeout(limit, response).await.ok()
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use futures_util::stream;
    use http_body_util::BodyExt as _;

    use super::*;

    #[tokio::test]
    async fn an_empty_body_is_sent_at_once() {
        let (_upload, sent) = Upload::new(Body::empty());
        tokio::time::timeout(Duration::from_millis(100), sent)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_body_is_sent_after_its_last_frame() {
        let chunks = stream::iter([
            Ok::<_, std::io::Error>(Bytes::from("a")),
            Ok(Bytes::from("b")),
        ]);
        let (upload, sent) = Upload::new(Body::from_stream(chunks));
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let watcher = tokio::spawn(async move {
            sent.await;
            flag.store(true, Ordering::SeqCst);
        });
        tokio::task::yield_now().await;
        assert!(!done.load(Ordering::SeqCst));
        let body = upload.collect().await.unwrap().to_bytes();
        assert_eq!(body.as_ref(), b"ab");
        watcher.await.unwrap();
        assert!(done.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn the_limit_counts_from_the_end_of_the_upload() {
        let limit = Duration::from_secs(1);
        // The upload takes 5 s, the response comes 0.5 s after it.
        let (tx, rx) = oneshot::channel::<()>();
        let sent = async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let _ = tx.send(());
        };
        let response = async move {
            let _ = rx.await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            "response"
        };
        // `sent` must run on its own: in the test it also drives the
        // response.
        let (sent_tx, sent_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            sent.await;
            let _ = sent_tx.send(());
        });
        let done = respond(
            response,
            async move {
                let _ = sent_rx.await;
            },
            limit,
        )
        .await;
        assert_eq!(done, Some("response"));

        // No response after the upload: the limit.
        let never = std::future::pending::<()>();
        assert_eq!(respond(never, async {}, limit).await, None);
        // An early response while the upload still runs.
        let early = async { "early" };
        assert_eq!(
            respond(early, std::future::pending(), limit).await,
            Some("early")
        );
    }

    #[tokio::test]
    async fn the_connect_deadline_covers_the_whole_connection() {
        #[derive(Clone)]
        struct Stalls;
        impl tower_service::Service<Uri> for Stalls {
            type Response = ();
            type Error = std::io::Error;
            type Future = std::future::Pending<Result<(), std::io::Error>>;
            fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }
            fn call(&mut self, _: Uri) -> Self::Future {
                std::future::pending()
            }
        }
        let mut deadline = Deadline {
            inner: Stalls,
            within: Duration::from_millis(50),
        };
        let error =
            tower_service::Service::call(&mut deadline, Uri::from_static("http://app.test"))
                .await
                .unwrap_err();
        assert!(error.is::<ConnectTimeout>(), "{error}");
    }
}
