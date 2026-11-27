//! The plain HTTP listener with TLS on (`ROEDY_HTTP_LISTEN`): ACME HTTP-01
//! challenges under `/.well-known/acme-challenge/<token>` (RFC 8555 § 8.3)
//! and `308` to the same path and query under `ROEDY_PUBLIC_URL` for
//! everything else. No web app, no API on plain HTTP.

use std::sync::Arc;

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse as _, Response},
};
use tower_http::trace::TraceLayer;

use super::Challenges;
use crate::logging;

const CHALLENGE_PREFIX: &str = "/.well-known/acme-challenge/";

#[derive(Clone)]
struct Redirect {
    /// `https://host[:port]`, without a trailing slash.
    origin: Arc<str>,
    challenges: Arc<Challenges>,
}

/// The router of the HTTP listener.
pub fn router(public_url: &url::Url, challenges: Arc<Challenges>) -> Router {
    Router::new()
        .fallback(handle)
        .layer(TraceLayer::new_for_http().make_span_with(logging::request_span))
        .with_state(Redirect {
            origin: public_url.origin().ascii_serialization().into(),
            challenges,
        })
}

async fn handle(State(redirect): State<Redirect>, request: Request) -> Response {
    let path = request.uri().path();
    if let Some(token) = path.strip_prefix(CHALLENGE_PREFIX) {
        if !matches!(*request.method(), Method::GET | Method::HEAD) {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        return match redirect.challenges.http(token) {
            Some(key_authorization) => {
                tracing::info!("HTTP-01 validation answered");
                (
                    [
                        (header::CONTENT_TYPE, HeaderValue::from_static("text/plain")),
                        (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
                    ],
                    key_authorization,
                )
                    .into_response()
            }
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    let target = request.uri().path_and_query().map_or("/", |p| p.as_str());
    // An absolute-form request target may carry no leading slash.
    let target = if target.starts_with('/') { target } else { "/" };
    match HeaderValue::from_str(&format!("{}{target}", redirect.origin)) {
        Ok(location) => (
            StatusCode::PERMANENT_REDIRECT,
            [
                (header::LOCATION, location),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            ],
        )
            .into_response(),
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    use super::*;

    async fn send(
        router: &Router,
        method: Method,
        uri: &str,
    ) -> (StatusCode, Option<String>, String) {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_owned());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, location, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn redirects_everything_but_challenges() {
        let challenges = Arc::new(Challenges::default());
        let url = url::Url::parse("https://hub.example.test").unwrap();
        let router = router(&url, challenges.clone());
        for (method, uri, location) in [
            (Method::GET, "/", "https://hub.example.test/"),
            (
                Method::GET,
                "/login?next=%2Fmail",
                "https://hub.example.test/login?next=%2Fmail",
            ),
            (
                Method::POST,
                "/api/v1/auth/login",
                "https://hub.example.test/api/v1/auth/login",
            ),
            (
                Method::GET,
                "//evil.example/x",
                "https://hub.example.test//evil.example/x",
            ),
            (Method::GET, "/healthz", "https://hub.example.test/healthz"),
        ] {
            let (status, got, body) = send(&router, method, uri).await;
            assert_eq!(status, StatusCode::PERMANENT_REDIRECT, "{uri}");
            assert_eq!(got.as_deref(), Some(location), "{uri}");
            assert_eq!(body, "");
        }

        let (status, _, _) = send(&router, Method::GET, "/.well-known/acme-challenge/abc").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        challenges.add_http("abc", "abc.thumbprint");
        let (status, location, body) =
            send(&router, Method::GET, "/.well-known/acme-challenge/abc").await;
        assert_eq!(
            (status, location, body.as_str()),
            (StatusCode::OK, None, "abc.thumbprint")
        );
        let (status, _, _) = send(&router, Method::POST, "/.well-known/acme-challenge/abc").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);

        // A port in the public address stays.
        let url = url::Url::parse("https://hub.example.test:8443").unwrap();
        let (_, location, _) = send(&super::router(&url, challenges), Method::GET, "/x").await;
        assert_eq!(location.as_deref(), Some("https://hub.example.test:8443/x"));
    }
}
