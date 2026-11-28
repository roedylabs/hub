//! HTTP handlers: operational endpoints (`/healthz`, `/readyz`) and the API
//! under `/api/v1` with JSON errors, CSRF protection, sessions and
//! `Cache-Control: no-store`.
//!
//! Taken over from Roedy Mail (`crates/server/src/api/mod.rs`): the same
//! middleware order and CSRF rules, without maintenance mode.

pub mod apps;
pub mod audit;
pub mod auth;
pub mod branding;
pub mod email;
pub mod error;
pub mod health;
pub mod home;
pub mod invites;
pub mod me;
pub mod mfa;
pub mod organization;
pub mod password;
pub mod public;
pub mod session;
pub mod sessions;
pub mod settings;
pub mod setup;
pub mod stalwart;
pub mod system_mail;
pub mod users;

use axum::{
    Router,
    extract::{FromRequest, OptionalFromRequest, Request},
    http::{HeaderMap, HeaderValue, Method, header},
    middleware::{self, Next},
    response::{IntoResponse as _, Response},
};
use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use tower_http::set_header::SetResponseHeaderLayer;

pub use error::{ApiError, FieldErrors};

use crate::{app::AppState, auth::extract};

/// Path under which [`router`] is mounted.
pub const PREFIX: &str = "/api/v1";

/// Required header for modifying requests (CSRF protection through
/// `SameSite` + `X-Requested-With`). A foreign form cannot set it, and a
/// foreign `fetch` with a custom header needs a CORS preflight, which the API
/// does not allow.
pub const CSRF_HEADER: &str = "x-requested-with";

pub fn router(state: AppState) -> Router<AppState> {
    let routes = setup::routes()
        .merge(auth::routes())
        .merge(password::routes())
        .merge(email::routes())
        .merge(invites::routes())
        .merge(users::routes())
        .merge(apps::routes())
        .merge(audit::routes())
        .merge(organization::routes())
        .merge(branding::routes())
        .merge(public::routes())
        .merge(system_mail::routes())
        .merge(session::routes())
        .merge(home::routes())
        .merge(me::routes())
        .merge(mfa::routes())
        .merge(sessions::routes())
        .merge(settings::routes())
        .merge(stalwart::servers::routes())
        .merge(stalwart::accounts::routes());
    with_middleware(routes, state)
}

/// Wraps the API's fallback and middleware around `routes`. Tests put their
/// own routes behind the same stack this way.
pub fn with_middleware(routes: Router<AppState>, state: AppState) -> Router<AppState> {
    routes
        .fallback(not_found)
        // Order from inside to outside: session, CSRF (before any database
        // query), then `no-store` for every response, errors included.
        .layer(middleware::from_fn_with_state(state, extract::resolve))
        .layer(middleware::from_fn(require_csrf_header))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}

/// 404 `not_found` as JSON, also for `/internal/…` on the public listener.
pub async fn not_found() -> ApiError {
    ApiError::NotFound
}

/// Rejects modifying requests without a non-empty `X-Requested-With`, and
/// reading ones the browser loads as an image, stylesheet, font or the like:
/// a page of an app on the same origin could otherwise embed
/// `<img src="/api/v1/…">` and send the hub's cookie. The web app calls the
/// API only via `fetch` (`empty`) and follows links as a `document`.
pub async fn require_csrf_header(request: Request, next: Next) -> Response {
    let safe = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if safe && is_subresource(request.headers()) {
        return ApiError::Csrf.into_response();
    }
    if safe || has_csrf_header(request.headers()) {
        next.run(request).await
    } else {
        ApiError::Csrf.into_response()
    }
}

/// For reading endpoints that still trigger something – such as counting
/// towards a limit: requires `X-Requested-With` like modifying requests
/// (otherwise 403 `csrf`). A foreign `<img>` or a navigation from a foreign
/// page sends the cookie (`SameSite=Lax`), but never this header. Call before
/// any limit.
pub fn require_requested_with(headers: &HeaderMap) -> Result<(), ApiError> {
    if has_csrf_header(headers) {
        Ok(())
    } else {
        Err(ApiError::Csrf)
    }
}

fn has_csrf_header(headers: &HeaderMap) -> bool {
    headers.get(CSRF_HEADER).is_some_and(|v| !v.is_empty())
}

/// `Sec-Fetch-Dest` of an embedded resource (Fetch Metadata). Browsers
/// without the header get through.
fn is_subresource(headers: &HeaderMap) -> bool {
    const SUBRESOURCES: [&str; 14] = [
        "image",
        "style",
        "font",
        "script",
        "audio",
        "video",
        "track",
        "object",
        "embed",
        "iframe",
        "frame",
        "xslt",
        "audioworklet",
        "paintworklet",
    ];
    headers
        .get("sec-fetch-dest")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|dest| SUBRESOURCES.iter().any(|d| dest.eq_ignore_ascii_case(d)))
}

/// For `PATCH` fields that can be missing (`None`), `null` (`Some(None)`) or
/// have a value: `#[serde(default, deserialize_with = "nullable")]`. Taken
/// over from Roedy Mail (`api::nullable`).
pub fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// JSON body of a request. If it does not fit, the API answers with 400
/// `bad_request` as JSON instead of axum's text response.
pub struct JsonBody<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for JsonBody<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        <axum::Json<T> as FromRequest<S>>::from_request(request, state)
            .await
            .map(|axum::Json(value)| Self(value))
            .map_err(|rejection| ApiError::BadRequest(rejection.body_text()))
    }
}

/// An optional body (`Option<JsonBody<T>>`): `None` without `Content-Type`,
/// otherwise JSON as for [`JsonBody`] (a body of another type gets 400
/// `bad_request` too).
impl<S: Send + Sync, T: DeserializeOwned> OptionalFromRequest<S> for JsonBody<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Option<Self>, Self::Rejection> {
        <axum::Json<T> as OptionalFromRequest<S>>::from_request(request, state)
            .await
            .map(|body| body.map(|axum::Json(value)| Self(value)))
            .map_err(|rejection| ApiError::BadRequest(rejection.body_text()))
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::StatusCode, routing::get};
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    use super::*;
    use crate::app::tests::state;

    /// The API with one test route under the real middleware.
    fn api(state: AppState) -> Router {
        let routes = Router::new().route(
            "/probe",
            get(|| async { "read" }).post(|| async { "written" }),
        );
        Router::new()
            .nest(PREFIX, with_middleware(routes, state.clone()))
            .with_state(state)
    }

    async fn send(router: &Router, request: Request<Body>) -> (StatusCode, Option<String>, String) {
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let cache = response
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().unwrap().to_owned());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, cache, String::from_utf8(body.to_vec()).unwrap())
    }

    fn request(method: Method, headers: &[(&str, &str)]) -> Request<Body> {
        let mut builder = Request::builder().method(method).uri("/api/v1/probe");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(Body::empty()).unwrap()
    }

    const CSRF: (StatusCode, Option<&str>, &str) = (
        StatusCode::FORBIDDEN,
        Some("no-store"),
        r#"{"error":"csrf"}"#,
    );

    fn owned(expected: (StatusCode, Option<&str>, &str)) -> (StatusCode, Option<String>, String) {
        (
            expected.0,
            expected.1.map(str::to_owned),
            expected.2.to_owned(),
        )
    }

    #[tokio::test]
    async fn modifying_requests_need_the_csrf_header() {
        let (state, _tmp) = state().await;
        let api = api(state);
        assert_eq!(send(&api, request(Method::POST, &[])).await, owned(CSRF));
        assert_eq!(
            send(&api, request(Method::POST, &[(CSRF_HEADER, "")])).await,
            owned(CSRF)
        );
        assert_eq!(
            send(&api, request(Method::POST, &[(CSRF_HEADER, "fetch")])).await,
            owned((StatusCode::OK, Some("no-store"), "written"))
        );
        // A form of another page cannot set the header, whatever it posts.
        for method in [Method::PUT, Method::PATCH, Method::DELETE] {
            assert_eq!(
                send(&api, request(method, &[])).await.0,
                StatusCode::FORBIDDEN
            );
        }
    }

    #[tokio::test]
    async fn reading_requests_must_not_be_subresources() {
        let (state, _tmp) = state().await;
        let api = api(state);
        assert_eq!(
            send(&api, request(Method::GET, &[])).await,
            owned((StatusCode::OK, Some("no-store"), "read"))
        );
        for dest in ["empty", "document"] {
            let response = send(&api, request(Method::GET, &[("sec-fetch-dest", dest)])).await;
            assert_eq!(response.0, StatusCode::OK, "{dest}");
        }
        for dest in ["image", "IMAGE", "style", "script", "iframe", "font"] {
            assert_eq!(
                send(&api, request(Method::GET, &[("sec-fetch-dest", dest)])).await,
                owned(CSRF),
                "{dest}"
            );
        }
        // The header does not help an embedded resource.
        let embedded = request(
            Method::GET,
            &[("sec-fetch-dest", "image"), (CSRF_HEADER, "fetch")],
        );
        assert_eq!(send(&api, embedded).await, owned(CSRF));
    }

    #[tokio::test]
    async fn unknown_paths_are_json_404() {
        let (state, _tmp) = state().await;
        let api = api(state);
        let request = Request::get("/api/v1/missing").body(Body::empty()).unwrap();
        assert_eq!(
            send(&api, request).await,
            owned((
                StatusCode::NOT_FOUND,
                Some("no-store"),
                r#"{"error":"not_found"}"#
            ))
        );
    }

    #[test]
    fn requested_with_for_reading_endpoints() {
        let mut headers = HeaderMap::new();
        assert!(require_requested_with(&headers).is_err());
        headers.insert(CSRF_HEADER, HeaderValue::from_static("fetch"));
        assert!(require_requested_with(&headers).is_ok());
    }
}
