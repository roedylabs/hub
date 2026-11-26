//! The internal API for the apps (docs/app-contract.md › Internal API) on
//! the internal listener. So far only `GET /internal/v1/jwks`; pairing,
//! manifest, users and events follow.

use axum::{
    Json, Router,
    http::{HeaderValue, header},
    response::{IntoResponse as _, Response},
    routing::get,
};
use jiff::Timestamp;

use crate::app::AppState;

/// Prefix of the internal API, version 1.
pub const PREFIX: &str = "/internal/v1";

/// The routes under [`PREFIX`]; `app::internal_router` adds the fallback.
pub fn router() -> Router<AppState> {
    Router::new().route(&format!("{PREFIX}/jwks"), get(jwks))
}

/// `GET /internal/v1/jwks`: the public signing keys, without
/// authentication, cacheable for 5 minutes (contract § 4.4 and § 7).
async fn jwks(axum::extract::State(state): axum::extract::State<AppState>) -> Response {
    (
        [(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=300"),
        )],
        Json(state.keys.jwks(Timestamp::now())),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
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
