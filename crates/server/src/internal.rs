//! The internal API for the apps (docs/app-contract.md › Internal API) on
//! the internal listener:
//!
//! - `GET /internal/v1/jwks`: the public signing keys, without
//!   authentication.
//! - `POST /internal/v1/pair` ([`pair`]): pairing with a code.
//! - `PUT /internal/v1/manifest` ([`pair::put_manifest`]): the manifest on
//!   every start of the app.
//! - `GET /internal/v1/users[/<sub>]` ([`users`]): the users with access.
//! - `GET /internal/v1/events` ([`events`]): catching up on events.
//!
//! Every endpoint but `pair` and `jwks` needs the app secret
//! ([`PairedApp`]). Responses are JSON with `Cache-Control: no-store` unless
//! stated otherwise; bodies are limited to [`MAX_BODY`].

pub mod events;
pub mod pair;
pub mod users;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, FromRequestParts, State},
    http::{HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse as _, Response},
    routing::{get, post, put},
};
use jiff::Timestamp;
use roedy_hub_contract::pairing::APP_SECRET_PREFIX;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::{
    api::ApiError,
    app::AppState,
    apps,
    db::{self, repo::apps::App},
};

/// Prefix of the internal API, version 1.
pub const PREFIX: &str = "/internal/v1";

/// Largest request body: a manifest with its icon (at most 64 KiB) fits
/// many times over.
pub const MAX_BODY: usize = 256 * 1024;

/// Longest `Authorization` value that can hold a secret.
const MAX_SECRET_LEN: usize = 128;

/// The routes under [`PREFIX`]; `app::internal_router` adds the fallback.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(&format!("{PREFIX}/jwks"), get(jwks))
        .route(&format!("{PREFIX}/pair"), post(pair::pair))
        .route(&format!("{PREFIX}/manifest"), put(pair::put_manifest))
        .route(&format!("{PREFIX}/users"), get(users::list))
        .route(&format!("{PREFIX}/users/{{sub}}"), get(users::show))
        .route(&format!("{PREFIX}/events"), get(events::list))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}

/// `GET /internal/v1/jwks`: the public signing keys, without
/// authentication, cacheable for 5 minutes (contract § 4.4 and § 7).
async fn jwks(State(state): State<AppState>) -> Response {
    (
        [(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=300"),
        )],
        Json(state.keys.jwks(Timestamp::now())),
    )
        .into_response()
}

/// `401 invalid_app_credentials`: unknown or revoked secret.
pub fn invalid_credentials() -> ApiError {
    ApiError::rejected(StatusCode::UNAUTHORIZED, "invalid_app_credentials")
}

/// The paired app whose secret the request carries
/// (`Authorization: Bearer rhs_…`), and the secret's hash – a change checks
/// it again in its transaction, since pairing anew replaces the secret;
/// otherwise [`invalid_credentials`]. Records the contact
/// (`apps.contacted_at`, at most once a minute).
#[derive(Debug, Clone)]
pub struct PairedApp(pub App, pub [u8; 32]);

impl FromRequestParts<AppState> for PairedApp {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let secret = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| {
                let (scheme, secret) = value.trim().split_once(' ')?;
                scheme.eq_ignore_ascii_case("bearer").then(|| secret.trim())
            })
            .filter(|secret| {
                secret.starts_with(APP_SECRET_PREFIX) && secret.len() <= MAX_SECRET_LEN
            })
            .ok_or_else(invalid_credentials)?;
        let hash = apps::hash(secret);
        db::call(&state.db, move |conn| {
            let app = db::repo::apps::by_secret_hash(conn, &hash)?;
            // "Last contact" in the administration.
            if let Some(app) = &app {
                db::repo::apps::touch_contact(conn, &app.id)?;
            }
            Ok(app)
        })
        .await?
        .map(|app| Self(app, hash))
        .ok_or_else(invalid_credentials)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt as _;
    use roedy_hub_contract::Jwks;
    use tower::ServiceExt as _;

    use crate::app::{internal_router, tests::state};

    #[tokio::test]
    async fn jwks_lists_the_published_keys() {
        let (state, _tmp) = state().await;
        let signer = state.keys.signer(jiff::Timestamp::now()).unwrap();
        let response = internal_router(state)
            .oneshot(
                Request::get("/internal/v1/jwks")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let headers = response.headers().clone();
        assert_eq!(headers["cache-control"], "max-age=300");
        assert_eq!(headers["content-type"], "application/json");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "keys": [{
                "kty": "OKP",
                "crv": "Ed25519",
                "x": signer.jwk().x.unwrap(),
                "kid": signer.kid(),
                "use": "sig",
                "alg": "EdDSA",
            }] })
        );
        let jwks: Jwks = serde_json::from_slice(&body).unwrap();
        assert_eq!(jwks.find(signer.kid()), Some(signer.public_key()));
    }

    #[tokio::test]
    async fn the_public_listener_never_serves_the_jwks() {
        let (state, _tmp) = state().await;
        let response = crate::app::router(state)
            .oneshot(
                Request::get("/internal/v1/jwks")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }
}
