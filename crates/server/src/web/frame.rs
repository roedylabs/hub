//! `GET /api/v1/frame.js`: the hub's frame, the web components every app
//! shows the same way (docs/app-contract.md › The hub's frame; for now the app
//! switcher), as a dependency-free ES module from the web app's build
//! (`apps/web/src/frame`, written to `_hub/frame.js` by `pnpm build`).
//!
//! Public and outside the API's middleware: the browser loads it with
//! `<script>` (which the API refuses as a subresource), also before sign-in.
//! Apps include it at this fixed address, so it carries no version: the
//! browser revalidates it via the ETag (`no-cache`, then mostly 304), and a
//! new hub version reaches every app on its next load. Same origin only
//! (`Cross-Origin-Resource-Policy`): the frame reads the hub's session and
//! has no use on other sites. Without a web app (development with `pnpm dev`)
//! `404 {"error": "not_found"}`.

use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, header},
    response::{IntoResponse as _, Response},
};

use super::static_files::{CACHE_REVALIDATE, cached};
use crate::{api::ApiError, app::AppState};

/// The path, for the router.
pub const SCRIPT_PATH: &str = "/api/v1/frame.js";

/// Where the build puts it (apps/web/vite.frame.config.ts).
const FILE: &str = "_hub/frame.js";

const CONTENT_TYPE: &str = "text/javascript; charset=utf-8";

pub async fn script(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(asset) = state.web.get(FILE).await else {
        return (
            [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
            ApiError::NotFound,
        )
            .into_response();
    };
    let mut response = cached(
        &headers,
        &asset.etag,
        CACHE_REVALIDATE,
        CONTENT_TYPE,
        asset.body,
    );
    response.headers_mut().insert(
        header::HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    response
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
    };
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    use super::*;
    use crate::{
        app::{router, tests::state_with_web},
        web::static_files::WebAssets,
    };

    const SCRIPT: &str = "customElements.define('roedy-app-switcher',class extends HTMLElement{});";

    async fn send(
        web: WebAssets,
        method: Method,
        headers: &[(&str, &str)],
    ) -> (StatusCode, HeaderMap, String) {
        let (state, _tmp) = state_with_web(web).await;
        let mut request = Request::builder().method(method).uri(SCRIPT_PATH);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = router(state)
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (status, headers) = (response.status(), response.headers().clone());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, headers, String::from_utf8(body.to_vec()).unwrap())
    }

    fn build() -> (WebAssets, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("_hub")).unwrap();
        std::fs::write(root.join("index.html"), "<!doctype html>").unwrap();
        std::fs::write(root.join(FILE), SCRIPT).unwrap();
        (WebAssets::new(Some(root)).unwrap(), tmp)
    }

    fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
        headers.get(name).map(|value| value.to_str().unwrap())
    }

    #[tokio::test]
    async fn serves_the_script_without_sign_in_and_revalidated() {
        let (web, _build) = build();
        // As the browser loads it: `<script type="module">`, a subresource.
        let (status, headers, body) = send(
            web.clone(),
            Method::GET,
            &[("sec-fetch-dest", "script"), ("sec-fetch-mode", "cors")],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, SCRIPT);
        assert_eq!(header(&headers, "content-type"), Some(CONTENT_TYPE));
        assert_eq!(header(&headers, "cache-control"), Some("no-cache"));
        assert_eq!(
            header(&headers, "cross-origin-resource-policy"),
            Some("same-origin")
        );
        assert_eq!(header(&headers, "x-content-type-options"), Some("nosniff"));
        let etag = header(&headers, "etag").unwrap().to_owned();

        // Unchanged: 304 with the same caching.
        let (status, headers, body) =
            send(web.clone(), Method::GET, &[("if-none-match", &etag)]).await;
        assert_eq!(status, StatusCode::NOT_MODIFIED);
        assert!(body.is_empty());
        assert_eq!(header(&headers, "cache-control"), Some("no-cache"));
        assert_eq!(header(&headers, "etag"), Some(etag.as_str()));

        let (status, _, body) = send(web, Method::HEAD, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn not_found_without_a_web_app_or_frame() {
        // A build from before the frame.
        let old = tempfile::tempdir().unwrap();
        std::fs::write(old.path().join("index.html"), "<!doctype html>").unwrap();
        for web in [WebAssets::None, WebAssets::new(Some(old.path())).unwrap()] {
            let (status, headers, body) = send(web, Method::GET, &[]).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(body, r#"{"error":"not_found"}"#);
            assert_eq!(header(&headers, "cache-control"), Some("no-store"));
        }
    }
}
