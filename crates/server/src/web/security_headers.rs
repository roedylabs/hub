//! Security headers of the hub's own responses.
//!
//! - **CSP:** everything from the hub's origin only. SvelteKit starts the SPA
//!   with an inline script in `index.html`; `static_files` attaches its
//!   SHA-256 to the response as [`InlineScripts`].
//! - `Referrer-Policy: no-referrer` (links from mails reveal no hub
//!   addresses), `Permissions-Policy` without camera, microphone etc.,
//!   `X-Content-Type-Options: nosniff`, `Cross-Origin-Opener-Policy:
//!   same-origin` (a foreign page that opens the hub keeps no handle on its
//!   window).
//! - **HSTS** only with `ROEDY_TRUST_PROXY`: then a reverse proxy terminates
//!   TLS; without one the server cannot know whether it is reachable over
//!   HTTPS.
//!
//! If a handler sets a CSP itself, its own stays. The gateway's responses
//! from the apps (later) do not pass through this layer: each app sends its
//! own headers (docs/app-contract.md › Response headers).
//!
//! Taken over from Roedy Mail (`crates/server/src/web/security_headers.rs`)
//! without the `connect-src` from the mail server registry and without the
//! sources for HTML mails (`img-src https:`, `frame-src blob:`).

use axum::{
    extract::{Request, State},
    http::{HeaderName, HeaderValue, header},
    middleware::Next,
    response::Response,
};
use base64::Engine as _;
use sha2::{Digest as _, Sha256};

use crate::app::AppState;

const PERMISSIONS_POLICY: &str = "camera=(), display-capture=(), geolocation=(), \
     microphone=(), midi=(), payment=(), serial=(), usb=(), xr-spatial-tracking=()";
const HSTS: &str = "max-age=31536000";
static PERMISSIONS_POLICY_HEADER: HeaderName = HeaderName::from_static("permissions-policy");
static OPENER_POLICY_HEADER: HeaderName = HeaderName::from_static("cross-origin-opener-policy");

/// The CSP with the given script hashes in `script-src`.
pub fn policy(scripts: &[String]) -> HeaderValue {
    let mut script_src = String::from("'self'");
    for hash in scripts {
        script_src.push(' ');
        script_src.push_str(hash);
    }
    // `style-src 'unsafe-inline'`: Svelte sets styles in attributes
    // (transitions, positions). `img-src data: blob:`: inline icons and the
    // preview of an uploaded logo.
    let policy = format!(
        "default-src 'self'; script-src {script_src}; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; connect-src 'self'; object-src 'none'; base-uri 'self'; \
         form-action 'self'; frame-ancestors 'none'"
    );
    // The hashes are Base64.
    HeaderValue::from_str(&policy)
        .unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'"))
}

/// Hashes of the inline scripts of an HTML document (`'sha256-…'`), as a
/// response extension.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineScripts(pub Vec<String>);

impl InlineScripts {
    /// Finds `<script>` elements without `src` and hashes their content as the
    /// browser sees it (text between the tags, unchanged).
    pub fn of_html(html: &[u8]) -> Self {
        let lower = html.to_ascii_lowercase();
        let mut hashes = Vec::new();
        let mut rest = 0;
        while let Some(at) = find(&lower[rest..], b"<script") {
            let tag_start = rest + at;
            let after_name = tag_start + b"<script".len();
            let Some(end) = find(&lower[after_name..], b">") else {
                break;
            };
            let tag_end = after_name + end;
            let attributes = &lower[after_name..tag_end];
            rest = tag_end + 1;
            if !attributes.first().is_none_or(u8::is_ascii_whitespace) {
                // `<scripts>` or similar.
                continue;
            }
            let Some(close) = find(&lower[rest..], b"</script") else {
                break;
            };
            let content = &html[rest..rest + close];
            rest += close;
            let external = attributes
                .split(u8::is_ascii_whitespace)
                .any(|token| token == b"src" || token.starts_with(b"src="));
            if external {
                continue;
            }
            let digest = Sha256::digest(content);
            let hash = format!(
                "'sha256-{}'",
                base64::engine::general_purpose::STANDARD.encode(digest)
            );
            if !hashes.contains(&hash) {
                hashes.push(hash);
            }
        }
        Self(hashes)
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Middleware for all of the hub's own responses.
pub async fn apply(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let scripts = response
        .extensions_mut()
        .remove::<InlineScripts>()
        .unwrap_or_default();
    let headers = response.headers_mut();
    if !headers.contains_key(header::CONTENT_SECURITY_POLICY) {
        headers.insert(header::CONTENT_SECURITY_POLICY, policy(&scripts.0));
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        PERMISSIONS_POLICY_HEADER.clone(),
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
    headers.insert(
        OPENER_POLICY_HEADER.clone(),
        HeaderValue::from_static("same-origin"),
    );
    if state.trust_proxy.enabled() {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static(HSTS),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{HeaderMap, StatusCode},
        middleware,
        response::IntoResponse as _,
        routing::get,
    };
    use tower::ServiceExt as _;

    use super::*;
    use crate::{app::tests::state, config::TrustProxy};

    fn hash(content: &str) -> String {
        format!(
            "'sha256-{}'",
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(content))
        )
    }

    #[test]
    fn hashes_inline_scripts_only() {
        let html = "<!doctype html><head>\
            <script src=\"/_hub/a.js\"></script>\
            <script data-src=\"x\">with();</script>\
            <SCRIPT type=\"module\">\n  start();\n</SCRIPT>\
            <scripts>no script</scripts>\
            <script>\n  start();\n</script>\
            <script>empty();</script></head>";
        assert_eq!(
            InlineScripts::of_html(html.as_bytes()),
            InlineScripts(vec![
                hash("with();"),
                hash("\n  start();\n"),
                hash("empty();")
            ])
        );
        assert_eq!(
            InlineScripts::of_html(b"<p>without</p>"),
            InlineScripts::default()
        );
        assert_eq!(
            InlineScripts::of_html(b"<script>open"),
            InlineScripts::default()
        );
    }

    #[test]
    fn known_hash_from_the_csp_specification() {
        assert_eq!(
            InlineScripts::of_html(b"<script>alert('Hello, world.');</script>").0,
            ["'sha256-qznLcsROx4GACP2dm0UCKCzCG+HiZ1guq6ZZDob/Tng='"]
        );
    }

    #[test]
    fn policy_allows_only_the_origin() {
        assert_eq!(
            policy(&["'sha256-abc='".into()]).to_str().unwrap(),
            "default-src 'self'; script-src 'self' 'sha256-abc='; \
             style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; \
             object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'"
        );
        assert!(
            policy(&[])
                .to_str()
                .unwrap()
                .contains("script-src 'self'; ")
        );
    }

    async fn headers_with(trust_proxy: TrustProxy, own_csp: bool) -> HeaderMap {
        let (mut state, _tmp) = state().await;
        state.trust_proxy = trust_proxy;
        let router = Router::new()
            .route(
                "/",
                get(move || async move {
                    let mut response = (StatusCode::OK, "page").into_response();
                    if own_csp {
                        response.headers_mut().insert(
                            header::CONTENT_SECURITY_POLICY,
                            HeaderValue::from_static("sandbox"),
                        );
                    }
                    response
                        .extensions_mut()
                        .insert(InlineScripts(vec!["'sha256-xyz='".into()]));
                    response
                }),
            )
            .layer(middleware::from_fn_with_state(state.clone(), apply))
            .with_state(state);
        let response = router
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(response.extensions().get::<InlineScripts>().is_none());
        response.headers().clone()
    }

    #[tokio::test]
    async fn sets_the_security_headers() {
        let headers = headers_with(TrustProxy(0), false).await;
        let get = |name: &str| headers.get(name).map(|v| v.to_str().unwrap().to_owned());
        assert!(
            get("content-security-policy")
                .unwrap()
                .contains("script-src 'self' 'sha256-xyz=';")
        );
        assert_eq!(get("x-content-type-options").as_deref(), Some("nosniff"));
        assert_eq!(get("referrer-policy").as_deref(), Some("no-referrer"));
        assert_eq!(
            get("permissions-policy").as_deref(),
            Some(PERMISSIONS_POLICY)
        );
        assert_eq!(
            get("cross-origin-opener-policy").as_deref(),
            Some("same-origin")
        );
        assert_eq!(get("strict-transport-security"), None);
    }

    #[tokio::test]
    async fn hsts_only_behind_a_trusted_proxy() {
        let headers = headers_with(TrustProxy(1), false).await;
        assert_eq!(
            headers.get(header::STRICT_TRANSPORT_SECURITY).unwrap(),
            HSTS
        );
    }

    #[tokio::test]
    async fn keeps_a_handlers_own_csp() {
        let headers = headers_with(TrustProxy(0), true).await;
        assert_eq!(
            headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
            "sandbox"
        );
    }
}
