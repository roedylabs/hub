//! `GET /api/v1/apps/<slug>/icon.svg`: an app's icon from its manifest, as
//! the hub sanitized it at pairing or with the last manifest
//! (docs/app-contract.md › Browser API of the hub).
//!
//! Public and outside the API's middleware: the browser loads it with
//! `<img>` (which the API refuses as a subresource) and without a session,
//! and it is cacheable. With the current version in `?v=` (`apps::icon_url`)
//! it stays in the cache for a year (`immutable`); any other address –
//! without `v`, or with the version of an earlier icon – is revalidated via
//! the ETag (`no-cache`), so that a stale URL never pins a new icon. Every
//! response carries the contract's CSP, so that the SVG cannot run anything
//! even when opened directly. Pending and unknown apps are `404
//! not_found`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse as _, Response},
};
use serde::Deserialize;

use super::static_files::{CACHE_IMMUTABLE, CACHE_REVALIDATE, cached};
use crate::{api::ApiError, app::AppState};

/// The path, for the router.
pub const ICON_PATH: &str = "/api/v1/apps/{slug}/icon.svg";

/// Contract § 6: no scripts, no external references, only inline styles.
pub const ICON_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

#[derive(Deserialize)]
pub struct Version {
    v: Option<String>,
}

pub async fn icon(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<Version>,
    headers: HeaderMap,
) -> Response {
    let Some((manifest, version)) = state
        .apps
        .get(&slug)
        .and_then(|app| Some((app.manifest.clone()?, app.icon_version.clone()?)))
    else {
        return (
            [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
            ApiError::NotFound,
        )
            .into_response();
    };
    let cache = if query.v.as_deref() == Some(version.as_str()) {
        CACHE_IMMUTABLE
    } else {
        CACHE_REVALIDATE
    };
    let mut response = cached(
        &headers,
        &format!("\"{version}\""),
        cache,
        "image/svg+xml",
        manifest.icon.clone(),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(ICON_CSP),
    );
    if response.status() == StatusCode::NOT_MODIFIED {
        response.headers_mut().remove(header::CONTENT_TYPE);
    }
    response
}
