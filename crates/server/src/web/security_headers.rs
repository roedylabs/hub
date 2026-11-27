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
//! - **HSTS** (one year, without `includeSubDomains` and `preload`) when the
//!   hub terminates TLS itself (`ROEDY_TLS`) or with `ROEDY_TRUST_PROXY`, when
//!   a reverse proxy does; otherwise the server cannot know whether it is
//!   reachable over HTTPS.
//!
//! If a handler sets a CSP itself, its own stays. The gateway's responses
//! from the apps (marked [`Proxied`]) pass through unchanged: each app sends
//! its own headers, the gateway applies the contract's rules
//! (docs/app-contract.md › Response headers, `gateway::headers`). The
//! gateway's own error responses get the hub's headers like every other
//! page of the hub.
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
/// Also for the gateway's responses (`gateway::headers`).
pub const HSTS: &str = "max-age=31536000";
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
    /// browser sees it: the text between the tags, with line endings
    /// normalized to LF as the HTML parser does. Quoted attribute values may
    /// contain `>`.
    pub fn of_html(html: &[u8]) -> Self {
        let lower = html.to_ascii_lowercase();
        let mut hashes = Vec::new();
        let mut rest = 0;
        while let Some(at) = find(&lower[rest..], b"<script") {
            let after_name = rest + at + b"<script".len();
            rest = after_name;
            if !lower
                .get(after_name)
                .is_some_and(|&b| b.is_ascii_whitespace() || b == b'>' || b == b'/')
            {
                // `<scripts>` or similar.
                continue;
            }
            let Some((names, tag_end)) = attribute_names(&lower, after_name) else {
                break;
            };
            rest = tag_end;
            let Some(close) = find(&lower[rest..], b"</script") else {
                break;
            };
            let content = &html[rest..rest + close];
            rest += close;
            if names.iter().any(|name| *name == b"src") {
                continue;
            }
            let digest = Sha256::digest(normalize_newlines(content));
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

/// Names of the attributes of the start tag from `at` (after the tag name)
/// and the position after its `>`, following the HTML tokenizer: values in
/// double or single quotes end only at the same quote. `None` if the tag does
/// not end.
fn attribute_names(lower: &[u8], mut at: usize) -> Option<(Vec<&[u8]>, usize)> {
    let mut names = Vec::new();
    loop {
        // Before the attribute name.
        while lower.get(at)?.is_ascii_whitespace() || lower[at] == b'/' {
            at += 1;
        }
        if lower[at] == b'>' {
            return Some((names, at + 1));
        }
        let start = at;
        // The first character may be `=` (it belongs to the name then).
        at += 1;
        while !matches!(lower.get(at)?, b'=' | b'>' | b'/') && !lower[at].is_ascii_whitespace() {
            at += 1;
        }
        names.push(&lower[start..at]);
        while lower.get(at)?.is_ascii_whitespace() {
            at += 1;
        }
        if lower[at] != b'=' {
            continue;
        }
        at += 1;
        while lower.get(at)?.is_ascii_whitespace() {
            at += 1;
        }
        match lower[at] {
            quote @ (b'"' | b'\'') => {
                at += 1 + find(&lower[at + 1..], &[quote])? + 1;
            }
            b'>' => return Some((names, at + 1)),
            _ => {
                while !lower.get(at)?.is_ascii_whitespace() && lower[at] != b'>' {
                    at += 1;
                }
            }
        }
    }
}

/// CRLF and lone CR become LF, as in the HTML parser's input stream.
fn normalize_newlines(content: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if !content.contains(&b'\r') {
        return content.into();
    }
    let mut normalized = Vec::with_capacity(content.len());
    let mut bytes = content.iter().peekable();
    while let Some(&byte) = bytes.next() {
        if byte == b'\r' {
            normalized.push(b'\n');
            if bytes.peek() == Some(&&b'\n') {
                bytes.next();
            }
        } else {
            normalized.push(byte);
        }
    }
    normalized.into()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Marks a response of an app that the gateway forwards: this layer leaves
/// it alone.
#[derive(Debug, Clone, Copy)]
pub struct Proxied;

/// Middleware for all of the hub's own responses.
pub async fn apply(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    if response.extensions().get::<Proxied>().is_some() {
        return response;
    }
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
    if state.hsts() {
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
    fn quoted_attributes_may_contain_angle_brackets_and_src() {
        let html = "<script data-note=\">\">start();</script>\
            <script data-x='a > b' type=module>second();</script>\
            <script data-y=\"src=x\">third();</script>\
            <script defer src=/_hub/a.js></script>\
            <script/>fourth();</script>";
        assert_eq!(
            InlineScripts::of_html(html.as_bytes()),
            InlineScripts(vec![
                hash("start();"),
                hash("second();"),
                hash("third();"),
                hash("fourth();")
            ])
        );
        // An unterminated quote or tag hashes nothing.
        assert_eq!(
            InlineScripts::of_html(b"<script data-a=\">x</script>"),
            InlineScripts::default()
        );
        assert_eq!(
            InlineScripts::of_html(b"<script "),
            InlineScripts::default()
        );
    }

    #[test]
    fn line_endings_are_normalized_like_the_browser() {
        let lf = hash("\nstart();\n");
        for html in [
            "<script>\nstart();\n</script>",
            "<script>\r\nstart();\r\n</script>",
            "<script>\rstart();\r</script>",
            "<script>\r\nstart();\r</script>",
        ] {
            assert_eq!(
                InlineScripts::of_html(html.as_bytes()).0,
                std::slice::from_ref(&lf),
                "{html:?}"
            );
        }
        assert_eq!(normalize_newlines(b"a\r\r\nb").as_ref(), b"a\n\nb");
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
