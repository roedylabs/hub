//! Header rules of the gateway as pure functions (docs/app-contract.md ›
//! Request headers the hub sends to the app, Response headers the hub
//! changes):
//!
//! - [`request_headers`]: what the app receives – hop-by-hop headers and
//!   every `X-Roedy-*` of the client removed, only the app's own cookies,
//!   `Host` and `X-Forwarded-*` from the hub's public URL, `Forwarded`
//!   removed, then the hub's `X-Roedy-Request-Id` and `X-Roedy-Identity`.
//! - [`forwarded_for`]: the trusted part of `X-Forwarded-For` plus the hub's
//!   TCP peer.
//! - [`response_headers`]: what the browser receives – hop-by-hop headers,
//!   `Set-Cookie` outside the app's namespace, `Clear-Site-Data`,
//!   `Service-Worker-Allowed` and the app's HSTS removed, the hub's HSTS and
//!   `X-Content-Type-Options: nosniff` added.

use std::net::IpAddr;

use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, header};

use crate::{client_ip, config::TrustProxy, web::security_headers::HSTS};

/// Every header with this prefix belongs to the hub.
const ROEDY_PREFIX: &[u8] = b"x-roedy-";
pub static REQUEST_ID: HeaderName = HeaderName::from_static("x-roedy-request-id");
/// `roedy_hub_contract::IDENTITY_HEADER` as a header name.
pub static IDENTITY: HeaderName = HeaderName::from_static("x-roedy-identity");
pub static X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
pub static X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
pub static X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");
static FORWARDED: HeaderName = HeaderName::from_static("forwarded");
static KEEP_ALIVE: HeaderName = HeaderName::from_static("keep-alive");
static PROXY_CONNECTION: HeaderName = HeaderName::from_static("proxy-connection");
static CLEAR_SITE_DATA: HeaderName = HeaderName::from_static("clear-site-data");
static SERVICE_WORKER_ALLOWED: HeaderName = HeaderName::from_static("service-worker-allowed");

/// Hop-by-hop headers (RFC 9110 § 7.6.1); the ones `Connection` names come
/// on top.
fn hop_by_hop() -> [&'static HeaderName; 6] {
    [
        &header::CONNECTION,
        &PROXY_CONNECTION,
        &KEEP_ALIVE,
        &header::TE,
        &header::TRANSFER_ENCODING,
        &header::UPGRADE,
    ]
}

/// Prefix of the cookies of the app `slug` (contract › Namespaces in the
/// browser): `__Host-roedy-<slug>_`.
pub fn cookie_prefix(slug: &str) -> String {
    format!("__Host-roedy-{slug}_")
}

/// Whether the request asks for a WebSocket (contract § 3.5): `GET` with
/// `Connection: upgrade` and `Upgrade: websocket` (tokens, any case).
pub fn is_websocket(method: &Method, headers: &HeaderMap) -> bool {
    method == Method::GET
        && has_token(headers, &header::CONNECTION, "upgrade")
        && has_token(headers, &header::UPGRADE, "websocket")
}

/// Whether a comma-separated header names `token` (ASCII case-insensitive).
fn has_token(headers: &HeaderMap, name: &HeaderName, token: &str) -> bool {
    tokens(headers, name).any(|t| t.eq_ignore_ascii_case(token.as_bytes()))
}

fn tokens<'a>(headers: &'a HeaderMap, name: &HeaderName) -> impl Iterator<Item = &'a [u8]> {
    headers
        .get_all(name)
        .iter()
        .flat_map(|value| value.as_bytes().split(|&b| b == b','))
        .map(<[u8]>::trim_ascii)
        .filter(|token| !token.is_empty())
}

/// Removes the hop-by-hop headers and the ones `Connection` names.
fn remove_hop_by_hop(headers: &mut HeaderMap) {
    let named: Vec<HeaderName> = tokens(headers, &header::CONNECTION)
        .filter_map(|token| HeaderName::from_bytes(token).ok())
        .collect();
    for name in named.iter().chain(hop_by_hop()) {
        headers.remove(name);
    }
}

/// What the hub adds to a forwarded request.
#[derive(Debug, Clone)]
pub struct Forward<'a> {
    /// The app's slug: its cookies go through.
    pub slug: &'a str,
    /// Public host (with a port other than the scheme's), from
    /// `ROEDY_PUBLIC_URL`.
    pub host: &'a HeaderValue,
    /// Public scheme, `https` or `http`.
    pub proto: &'a HeaderValue,
    /// [`forwarded_for`].
    pub forwarded_for: HeaderValue,
    pub request_id: HeaderValue,
    /// The signed identity, only with a hub session that may use the app.
    pub identity: Option<HeaderValue>,
    /// A WebSocket upgrade: `Connection` and `Upgrade` stay.
    pub websocket: bool,
}

/// The headers the app receives (contract § 3.2).
pub fn request_headers(incoming: &HeaderMap, forward: &Forward<'_>) -> HeaderMap {
    let mut headers = incoming.clone();
    remove_hop_by_hop(&mut headers);
    let roedy: Vec<HeaderName> = headers
        .keys()
        .filter(|name| name.as_str().as_bytes().starts_with(ROEDY_PREFIX))
        .cloned()
        .collect();
    for name in &roedy {
        headers.remove(name);
    }
    headers.remove(&FORWARDED);
    match app_cookies(incoming, forward.slug) {
        Some(cookies) => headers.insert(header::COOKIE, cookies),
        None => headers.remove(header::COOKIE),
    };
    headers.insert(header::HOST, forward.host.clone());
    headers.insert(X_FORWARDED_HOST.clone(), forward.host.clone());
    headers.insert(X_FORWARDED_PROTO.clone(), forward.proto.clone());
    headers.insert(X_FORWARDED_FOR.clone(), forward.forwarded_for.clone());
    if forward.websocket {
        headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    }
    headers.insert(REQUEST_ID.clone(), forward.request_id.clone());
    if let Some(identity) = &forward.identity {
        headers.insert(IDENTITY.clone(), identity.clone());
    }
    headers
}

/// The cookies of the app `slug` from every `Cookie` line, in one line;
/// `None` without any. Read per cookie, like the hub's own session cookie
/// (`auth::session::named_cookie`): bytes outside ASCII in one cookie do not
/// discard the others.
fn app_cookies(headers: &HeaderMap, slug: &str) -> Option<HeaderValue> {
    let prefix = cookie_prefix(slug);
    let mut joined: Vec<u8> = Vec::new();
    let pairs = headers
        .get_all(header::COOKIE)
        .iter()
        .flat_map(|value| value.as_bytes().split(|&b| b == b';'))
        .map(<[u8]>::trim_ascii)
        .filter(|pair| pair.starts_with(prefix.as_bytes()) && pair.contains(&b'='));
    for pair in pairs {
        if !joined.is_empty() {
            joined.extend_from_slice(b"; ");
        }
        joined.extend_from_slice(pair);
    }
    if joined.is_empty() {
        return None;
    }
    HeaderValue::from_bytes(&joined).ok()
}

/// `X-Forwarded-For` for the app: the entries the hub trusts (per its
/// `ROEDY_TRUST_PROXY`, the same ones `client_ip::resolve` reads), then the
/// hub's TCP peer. Entries further left could come from the client and are
/// dropped. The app sets its own `ROEDY_TRUST_PROXY` to the hub's plus one.
pub fn forwarded_for(peer: IpAddr, headers: &HeaderMap, trust: TrustProxy) -> String {
    let mut trusted = client_ip::trusted_entries(headers, trust);
    trusted.push(peer.to_canonical());
    trusted
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The app's response headers as the browser receives them (contract
/// § 3.3), and the names of the cookies dropped (for the log).
/// `switching`: the app's `101` to a WebSocket upgrade – `Connection` and
/// `Upgrade` stay. `hsts`: the hub runs behind a TLS proxy and sends its own
/// `Strict-Transport-Security`.
pub fn response_headers(
    mut headers: HeaderMap,
    slug: &str,
    switching: bool,
    hsts: bool,
) -> (HeaderMap, Vec<String>) {
    remove_hop_by_hop(&mut headers);
    if switching {
        headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    }
    let prefix = cookie_prefix(slug);
    let mut dropped = Vec::new();
    let cookies: Vec<HeaderValue> = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter(|value| {
            let name = set_cookie_name(value);
            let keep = name.starts_with(prefix.as_bytes());
            if !keep {
                dropped.push(String::from_utf8_lossy(name).into_owned());
            }
            keep
        })
        .cloned()
        .collect();
    headers.remove(header::SET_COOKIE);
    for cookie in cookies {
        headers.append(header::SET_COOKIE, cookie);
    }
    headers.remove(&CLEAR_SITE_DATA);
    headers.remove(&SERVICE_WORKER_ALLOWED);
    headers.remove(header::STRICT_TRANSPORT_SECURITY);
    if hsts {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static(HSTS),
        );
    }
    if !headers.contains_key(header::X_CONTENT_TYPE_OPTIONS) {
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
    }
    (headers, dropped)
}

/// Name of the cookie a `Set-Cookie` sets: before the first `=` of the part
/// before the first `;`, without surrounding whitespace (RFC 6265 § 5.2).
/// Without `=` the name is empty – no namespace, dropped.
fn set_cookie_name(value: &HeaderValue) -> &[u8] {
    let pair = value
        .as_bytes()
        .split(|&b| b == b';')
        .next()
        .unwrap_or_default();
    match pair.iter().position(|&b| b == b'=') {
        Some(at) => pair[..at].trim_ascii(),
        None => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_bytes(value.as_bytes()).unwrap(),
            );
        }
        headers
    }

    fn all(headers: &HeaderMap, name: &str) -> Vec<String> {
        headers
            .get_all(name)
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect()
    }

    fn forward(identity: Option<&str>, websocket: bool) -> HeaderMap {
        let host = HeaderValue::from_static("app.example.com");
        let proto = HeaderValue::from_static("https");
        let forward = Forward {
            slug: "mail",
            host: &host,
            proto: &proto,
            forwarded_for: HeaderValue::from_static("203.0.113.7, 10.0.0.2"),
            request_id: HeaderValue::from_static("01JDX5REQUEST000000000000"),
            identity: identity.map(|token| HeaderValue::from_str(token).unwrap()),
            websocket,
        };
        request_headers(&INCOMING.with(Clone::clone), &forward)
    }

    thread_local! {
        static INCOMING: HeaderMap = map(&[
            ("host", "internal.example.test"),
            ("x-roedy-identity", "forged.token.value"),
            ("X-Roedy-Request-Id", "forged"),
            ("x-roedy-anything", "1"),
            ("x-forwarded-host", "evil.example"),
            ("x-forwarded-proto", "http"),
            ("x-forwarded-for", "198.51.100.1"),
            ("forwarded", "for=198.51.100.1;proto=http"),
            ("connection", "keep-alive, x-custom-hop, Upgrade"),
            ("keep-alive", "timeout=5"),
            ("proxy-connection", "keep-alive"),
            ("te", "trailers"),
            ("transfer-encoding", "chunked"),
            ("upgrade", "websocket"),
            ("x-custom-hop", "1"),
            (
                "cookie",
                "__Host-roedy_hub_session=secret; __Host-roedy-mail_token=a; \
                 __Host-roedy-mail-archive_token=b; theme=dark",
            ),
            ("cookie", "__Host-roedy-mail_other=c; __Host-roedy-contacts_x=d"),
            ("origin", "https://app.example.com"),
            ("referer", "https://app.example.com/mail/"),
            ("sec-fetch-mode", "cors"),
            ("x-requested-with", "fetch"),
            ("authorization", "Bearer app-token"),
            ("accept-language", "de-DE"),
            ("content-length", "12"),
            ("expect", "100-continue"),
            ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
            ("sec-websocket-version", "13"),
        ]);
    }

    #[test]
    fn identity_is_the_contracts_header() {
        assert_eq!(
            IDENTITY,
            HeaderName::try_from(roedy_hub_contract::IDENTITY_HEADER).unwrap()
        );
    }

    #[test]
    fn request_headers_follow_the_contract() {
        let headers = forward(Some("hub.signed.token"), false);
        // (header, expected values) – one row per rule of § 3.2.
        let table: &[(&str, &[&str])] = &[
            // Forged hub headers are gone; the hub's own are set.
            ("x-roedy-identity", &["hub.signed.token"]),
            ("x-roedy-request-id", &["01JDX5REQUEST000000000000"]),
            ("x-roedy-anything", &[]),
            // Only the app's cookies, in one line; neither the hub's session
            // nor another app's (also not one whose slug starts the same).
            (
                "cookie",
                &["__Host-roedy-mail_token=a; __Host-roedy-mail_other=c"],
            ),
            ("host", &["app.example.com"]),
            ("x-forwarded-host", &["app.example.com"]),
            ("x-forwarded-proto", &["https"]),
            ("x-forwarded-for", &["203.0.113.7, 10.0.0.2"]),
            ("forwarded", &[]),
            // Hop-by-hop, also the one `Connection` names.
            ("connection", &[]),
            ("keep-alive", &[]),
            ("proxy-connection", &[]),
            ("te", &[]),
            ("transfer-encoding", &[]),
            ("upgrade", &[]),
            ("x-custom-hop", &[]),
            // Everything else unchanged.
            ("origin", &["https://app.example.com"]),
            ("referer", &["https://app.example.com/mail/"]),
            ("sec-fetch-mode", &["cors"]),
            ("x-requested-with", &["fetch"]),
            ("authorization", &["Bearer app-token"]),
            ("accept-language", &["de-DE"]),
            ("content-length", &["12"]),
            ("expect", &["100-continue"]),
            ("sec-websocket-key", &["dGhlIHNhbXBsZSBub25jZQ=="]),
        ];
        for (name, expected) in table {
            assert_eq!(all(&headers, name), *expected, "{name}");
        }
    }

    #[test]
    fn without_identity_and_for_websockets() {
        let headers = forward(None, false);
        assert!(headers.get("x-roedy-identity").is_none());
        assert_eq!(
            all(&headers, "x-roedy-request-id"),
            ["01JDX5REQUEST000000000000"]
        );

        let headers = forward(None, true);
        assert_eq!(all(&headers, "connection"), ["upgrade"]);
        assert_eq!(all(&headers, "upgrade"), ["websocket"]);
        assert_eq!(all(&headers, "sec-websocket-version"), ["13"]);
        // The other hop-by-hop headers still go.
        assert_eq!(all(&headers, "keep-alive"), Vec::<String>::new());
        assert_eq!(all(&headers, "x-custom-hop"), Vec::<String>::new());
    }

    #[test]
    fn cookies_of_the_app_only() {
        let table: &[(&[&str], Option<&str>)] = &[
            (&[], None),
            (&["__Host-roedy_hub_session=x"], None),
            (&["theme=dark; lang=de"], None),
            (&["__Host-roedy-mail_a=1"], Some("__Host-roedy-mail_a=1")),
            // Another slug that starts with this one.
            (&["__Host-roedy-mail-archive_a=1"], None),
            (&["__Host-roedy-mailx_a=1"], None),
            // The prefix is case-sensitive, like the browser's `__Host-`.
            (&["__host-roedy-mail_a=1"], None),
            // Leading whitespace of another kind than ASCII is part of the
            // name.
            (&["\u{2000}__Host-roedy-mail_a=1"], None),
            // Without a value it is no cookie.
            (&["__Host-roedy-mail_a"], None),
            (
                &["a=1;__Host-roedy-mail_a=1 ;b=ä", "__Host-roedy-mail_b=2"],
                Some("__Host-roedy-mail_a=1; __Host-roedy-mail_b=2"),
            ),
        ];
        for (lines, expected) in table {
            let mut headers = HeaderMap::new();
            for line in *lines {
                headers.append(
                    header::COOKIE,
                    HeaderValue::from_bytes(line.as_bytes()).unwrap(),
                );
            }
            let cookies = app_cookies(&headers, "mail");
            assert_eq!(
                cookies.as_ref().map(|v| v.to_str().unwrap()),
                *expected,
                "{lines:?}"
            );
        }
    }

    #[test]
    fn forwarded_for_keeps_the_trusted_entries() {
        let peer: IpAddr = "10.0.0.2".parse().unwrap();
        let table: &[(u8, &[&str], &str)] = &[
            // Without trusted proxies only the peer.
            (0, &[], "10.0.0.2"),
            (0, &["198.51.100.1"], "10.0.0.2"),
            // One proxy: its entry, not the client's own ones further left.
            (1, &["198.51.100.1, 203.0.113.7"], "203.0.113.7, 10.0.0.2"),
            (
                2,
                &["198.51.100.1, 203.0.113.7", "192.0.2.9"],
                "203.0.113.7, 192.0.2.9, 10.0.0.2",
            ),
            // Fewer entries than trusted: all of them.
            (3, &["203.0.113.7"], "203.0.113.7, 10.0.0.2"),
            // An unreadable entry ends the trusted part.
            (2, &["203.0.113.7, garbage"], "10.0.0.2"),
            (2, &["garbage, 203.0.113.7"], "203.0.113.7, 10.0.0.2"),
            // Ports and IPv4 in IPv6 as the hub reads them.
            (1, &["203.0.113.7:4711"], "203.0.113.7, 10.0.0.2"),
            (1, &["[2001:db8::1]:443"], "2001:db8::1, 10.0.0.2"),
            (1, &["::ffff:203.0.113.7"], "203.0.113.7, 10.0.0.2"),
        ];
        for (trust, lines, expected) in table {
            let mut headers = HeaderMap::new();
            for line in *lines {
                headers.append(X_FORWARDED_FOR.clone(), HeaderValue::from_static(line));
            }
            assert_eq!(
                forwarded_for(peer, &headers, TrustProxy(*trust)),
                *expected,
                "{trust} {lines:?}"
            );
        }
        let mapped: IpAddr = "::ffff:10.0.0.2".parse().unwrap();
        assert_eq!(
            forwarded_for(mapped, &HeaderMap::new(), TrustProxy(0)),
            "10.0.0.2"
        );
    }

    #[test]
    fn response_headers_follow_the_contract() {
        let incoming = map(&[
            ("content-type", "text/html"),
            (
                "set-cookie",
                "__Host-roedy-mail_token=a; Path=/; Secure; HttpOnly",
            ),
            (
                "set-cookie",
                "__Host-roedy_hub_session=forged; Path=/; Secure",
            ),
            ("set-cookie", "session=x; Path=/mail"),
            (
                "set-cookie",
                "__Host-roedy-mail-archive_token=b; Path=/; Secure",
            ),
            ("set-cookie", " __Host-roedy-mail_b = c ; Path=/; Secure"),
            ("set-cookie", "no-equals-sign"),
            ("clear-site-data", "\"*\""),
            ("service-worker-allowed", "/"),
            ("strict-transport-security", "max-age=0"),
            ("connection", "close, x-hop"),
            ("x-hop", "1"),
            ("keep-alive", "timeout=5"),
            ("transfer-encoding", "chunked"),
            ("upgrade", "h2c"),
            ("content-security-policy", "default-src 'self'"),
            ("cross-origin-opener-policy", "same-origin"),
            ("referrer-policy", "same-origin"),
            ("permissions-policy", "camera=()"),
            ("cache-control", "private"),
            ("x-roedy-from-app", "1"),
        ]);
        let (headers, dropped) = response_headers(incoming.clone(), "mail", false, true);
        let table: &[(&str, &[&str])] = &[
            (
                "set-cookie",
                &[
                    "__Host-roedy-mail_token=a; Path=/; Secure; HttpOnly",
                    " __Host-roedy-mail_b = c ; Path=/; Secure",
                ],
            ),
            ("clear-site-data", &[]),
            ("service-worker-allowed", &[]),
            ("strict-transport-security", &[HSTS]),
            ("connection", &[]),
            ("x-hop", &[]),
            ("keep-alive", &[]),
            ("transfer-encoding", &[]),
            ("upgrade", &[]),
            ("x-content-type-options", &["nosniff"]),
            // The app's own policies stay.
            ("content-type", &["text/html"]),
            ("content-security-policy", &["default-src 'self'"]),
            ("cross-origin-opener-policy", &["same-origin"]),
            ("referrer-policy", &["same-origin"]),
            ("permissions-policy", &["camera=()"]),
            ("cache-control", &["private"]),
            ("x-roedy-from-app", &["1"]),
        ];
        for (name, expected) in table {
            assert_eq!(all(&headers, name), *expected, "{name}");
        }
        assert_eq!(
            dropped,
            [
                "__Host-roedy_hub_session",
                "session",
                "__Host-roedy-mail-archive_token",
                ""
            ]
        );

        // Without a TLS proxy no HSTS at all; the app's `nosniff` stays.
        let mut incoming = incoming;
        incoming.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        let (headers, _) = response_headers(incoming, "mail", false, false);
        assert!(headers.get("strict-transport-security").is_none());
        assert_eq!(all(&headers, "x-content-type-options"), ["nosniff"]);
    }

    #[test]
    fn switching_protocols_keeps_the_upgrade() {
        let incoming = map(&[
            ("connection", "Upgrade"),
            ("upgrade", "websocket"),
            ("sec-websocket-accept", "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
        ]);
        let (headers, _) = response_headers(incoming, "mail", true, false);
        assert_eq!(all(&headers, "connection"), ["upgrade"]);
        assert_eq!(all(&headers, "upgrade"), ["websocket"]);
        assert_eq!(
            all(&headers, "sec-websocket-accept"),
            ["s3pPLMBiTxaQ9kYGzzhZRbK+xOo="]
        );
    }

    #[test]
    fn recognizes_websocket_upgrades() {
        type Row = (Method, &'static [(&'static str, &'static str)], bool);
        let table: &[Row] = &[
            (
                Method::GET,
                &[("connection", "Upgrade"), ("upgrade", "websocket")],
                true,
            ),
            (
                Method::GET,
                &[
                    ("connection", "keep-alive, upgrade"),
                    ("upgrade", "WebSocket"),
                ],
                true,
            ),
            (
                Method::GET,
                &[
                    ("connection", "keep-alive"),
                    ("connection", "Upgrade"),
                    ("upgrade", "websocket"),
                ],
                true,
            ),
            (Method::GET, &[("upgrade", "websocket")], false),
            (
                Method::GET,
                &[("connection", "upgrade"), ("upgrade", "h2c")],
                false,
            ),
            (
                Method::POST,
                &[("connection", "upgrade"), ("upgrade", "websocket")],
                false,
            ),
            (Method::GET, &[], false),
        ];
        for (method, pairs, expected) in table {
            assert_eq!(is_websocket(method, &map(pairs)), *expected, "{pairs:?}");
        }
    }
}
