//! An in-process ACME server (RFC 8555) with its own CA for the TLS tests:
//! plain HTTP on loopback, strict where the hub's client must be right
//! (nonces, JWS signatures, `url`, `kid`), and real validations – HTTP-01
//! fetches the key authorization from the hub's HTTP listener, TLS-ALPN-01
//! connects to its HTTPS listener with `acme-tls/1` and reads the challenge
//! certificate. Knobs ([`Knobs`]) inject `badNonce`, rate limits and error
//! documents, and set lifetimes and ARI windows.

#![allow(dead_code, reason = "each test uses its own part")]
#![allow(
    clippy::result_large_err,
    reason = "a handler's error is the response it sends"
)]

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse as _, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use jiff::{SignedDuration, Timestamp};
use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};
use roedy_hub_server::tls::{
    jose,
    key::P256Key,
    x509::{self, CertInfo, CertificateParams, CsrInfo, Extension},
};
use rustls::{
    ClientConfig, DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::ring::default_provider,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

const CA_KEY_ID: [u8; 20] = [0xca; 20];

/// What the test can change while the server runs.
#[derive(Debug, Clone)]
pub struct Knobs {
    /// Challenge types offered per authorization, in this order.
    pub offer: Vec<&'static str>,
    /// The next this many signed requests get `badNonce`.
    pub bad_nonces: usize,
    /// The next this many new orders get `rateLimited` with this
    /// `Retry-After` in seconds.
    pub rate_limited: usize,
    pub retry_after: u64,
    /// The next finalize gets this error document type.
    pub finalize_error: Option<&'static str>,
    /// Polls of an order that answer `processing` after finalize.
    pub processing_polls: usize,
    /// Validity of new certificates: start relative to now, lifetime.
    pub backdate: SignedDuration,
    pub lifetime: SignedDuration,
    /// ARI's window relative to now; `None`: no `renewalInfo` in the
    /// directory.
    pub ari: Option<(SignedDuration, SignedDuration)>,
    /// Validations of these types fail.
    pub fail: Vec<&'static str>,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            offer: vec!["http-01", "tls-alpn-01"],
            bad_nonces: 0,
            rate_limited: 0,
            retry_after: 1,
            finalize_error: None,
            processing_polls: 0,
            backdate: SignedDuration::from_hours(1),
            lifetime: SignedDuration::from_hours(90 * 24),
            ari: Some((
                SignedDuration::from_hours(60 * 24),
                SignedDuration::from_hours(62 * 24),
            )),
            fail: Vec::new(),
        }
    }
}

/// What happened, for the assertions.
#[derive(Debug, Clone, Default)]
pub struct Log {
    pub accounts: Vec<String>,
    pub contacts: Vec<Vec<String>>,
    pub bad_nonces_sent: usize,
    pub rate_limits_sent: usize,
    /// `replaces` of each new order (`None` without).
    pub orders: Vec<Option<String>>,
    /// Validations: challenge type and whether it passed.
    pub validations: Vec<(String, bool)>,
    pub issued: usize,
    pub ari_requests: usize,
}

struct Account {
    url: String,
    jwk: Value,
    thumbprint: String,
}

struct Challenge {
    kind: String,
    token: String,
    status: String,
    error: Option<Value>,
}

struct Authz {
    domain: String,
    status: String,
    challenges: Vec<Challenge>,
}

struct Order {
    account: String,
    domain: String,
    status: String,
    authz: Vec<String>,
    certificate: Option<String>,
    error: Option<Value>,
    processing_left: usize,
}

struct Inner {
    knobs: Knobs,
    log: Log,
    nonces: HashSet<String>,
    next_id: u64,
    accounts: Vec<Account>,
    authz: HashMap<String, Authz>,
    orders: HashMap<String, Order>,
    certificates: HashMap<String, String>,
    /// Where the hub listens: HTTP (HTTP-01) and HTTPS (TLS-ALPN-01).
    http_target: Option<SocketAddr>,
    tls_target: Option<SocketAddr>,
}

#[derive(Clone)]
pub struct FakeAcme {
    pub base: String,
    inner: Arc<Mutex<Inner>>,
    ca_key: Arc<P256Key>,
    pub ca_cert: Vec<u8>,
}

impl FakeAcme {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let ca_key = P256Key::generate().unwrap();
        let now = Timestamp::now();
        let ca_name = x509::name(Some("Fake ACME CA"));
        let ca_cert = CertificateParams {
            serial: vec![1],
            issuer: ca_name.clone(),
            subject: ca_name,
            not_before: now - SignedDuration::from_hours(24),
            not_after: now + SignedDuration::from_hours(24 * 3650),
            spki: ca_key.spki(),
            extensions: vec![Extension::ca(), Extension::subject_key_id(&CA_KEY_ID)],
        }
        .sign(&ca_key)
        .unwrap();
        let fake = Self {
            base,
            inner: Arc::new(Mutex::new(Inner {
                knobs: Knobs::default(),
                log: Log::default(),
                nonces: HashSet::new(),
                next_id: 1,
                accounts: Vec::new(),
                authz: HashMap::new(),
                orders: HashMap::new(),
                certificates: HashMap::new(),
                http_target: None,
                tls_target: None,
            })),
            ca_key: Arc::new(ca_key),
            ca_cert,
        };
        let router = Router::new()
            .route("/directory", get(directory))
            .route("/new-nonce", get(new_nonce).head(new_nonce))
            .route("/new-account", post(new_account))
            .route("/account/{id}", post(account))
            .route("/new-order", post(new_order))
            .route("/order/{id}", post(order))
            .route("/order/{id}/finalize", post(finalize))
            .route("/authz/{id}", post(authz))
            .route("/challenge/{id}/{index}", post(challenge))
            .route("/certificate/{id}", post(certificate))
            .route("/renewal-info/{id}", get(renewal_info))
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        fake
    }

    pub fn directory_url(&self) -> String {
        format!("{}/directory", self.base)
    }

    pub fn targets(&self, http: Option<SocketAddr>, tls: Option<SocketAddr>) {
        let mut inner = self.lock();
        inner.http_target = http;
        inner.tls_target = tls;
    }

    pub fn knobs(&self, change: impl FnOnce(&mut Knobs)) {
        change(&mut self.lock().knobs);
    }

    pub fn log(&self) -> Log {
        self.lock().log.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn nonce(&self) -> String {
        let mut inner = self.lock();
        let nonce = format!("nonce-{}", inner.next_id);
        inner.next_id += 1;
        inner.nonces.insert(nonce.clone());
        nonce
    }

    fn id(&self) -> String {
        let mut inner = self.lock();
        inner.next_id += 1;
        inner.next_id.to_string()
    }

    /// A response with a fresh nonce.
    fn reply(&self, status: StatusCode, location: Option<String>, body: Value) -> Response {
        let mut response = (status, axum::Json(body)).into_response();
        let headers = response.headers_mut();
        headers.insert(
            "replay-nonce",
            HeaderValue::from_str(&self.nonce()).unwrap(),
        );
        if let Some(location) = location {
            headers.insert(header::LOCATION, HeaderValue::from_str(&location).unwrap());
        }
        response
    }

    fn problem(&self, status: StatusCode, kind: &str, detail: &str) -> Response {
        let mut response = self.reply(
            status,
            None,
            json!({ "type": format!("urn:ietf:params:acme:error:{kind}"), "detail": detail }),
        );
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        response
    }

    /// Checks a flattened JWS: nonce, `url`, signature by the JWK (new
    /// account) or the account's key (`kid`). Returns the payload (`Null`
    /// for POST-as-GET) and the account's index or the JWK.
    fn verify(
        &self,
        uri: &Uri,
        body: &[u8],
        new_account: bool,
    ) -> Result<(Value, Signer), Response> {
        let malformed = |detail: &str| self.problem(StatusCode::BAD_REQUEST, "malformed", detail);
        let jws: Value = serde_json::from_slice(body).map_err(|_| malformed("not JSON"))?;
        let (Some(protected), Some(payload), Some(signature)) = (
            jws["protected"].as_str(),
            jws["payload"].as_str(),
            jws["signature"].as_str(),
        ) else {
            return Err(malformed("not a flattened JWS"));
        };
        let header: Value = URL_SAFE_NO_PAD
            .decode(protected)
            .ok()
            .and_then(|h| serde_json::from_slice(&h).ok())
            .ok_or_else(|| malformed("protected header"))?;
        if header["alg"] != "ES256" {
            return Err(self.problem(
                StatusCode::BAD_REQUEST,
                "badSignatureAlgorithm",
                "ES256 only",
            ));
        }
        // Nonce: always used up, then maybe a forced `badNonce`.
        let nonce_ok = header["nonce"]
            .as_str()
            .is_some_and(|nonce| self.lock().nonces.remove(nonce));
        let force = {
            let mut inner = self.lock();
            let force = inner.knobs.bad_nonces > 0;
            if force {
                inner.knobs.bad_nonces -= 1;
                inner.log.bad_nonces_sent += 1;
            }
            force
        };
        if !nonce_ok || force {
            return Err(self.problem(StatusCode::BAD_REQUEST, "badNonce", "stale nonce"));
        }
        if header["url"].as_str() != Some(self.url(uri.path()).as_str()) {
            return Err(self.problem(StatusCode::UNAUTHORIZED, "unauthorized", "url mismatch"));
        }
        let signer = if new_account {
            if header.get("kid").is_some() {
                return Err(malformed("newAccount needs jwk"));
            }
            Signer::Jwk(header["jwk"].clone())
        } else {
            if header.get("jwk").is_some() {
                return Err(malformed("jwk and kid"));
            }
            let kid = header["kid"].as_str().unwrap_or_default();
            let inner = self.lock();
            let index = inner
                .accounts
                .iter()
                .position(|a| a.url == kid)
                .ok_or_else(|| {
                    drop(inner);
                    self.problem(
                        StatusCode::BAD_REQUEST,
                        "accountDoesNotExist",
                        "unknown kid",
                    )
                })?;
            Signer::Account(index)
        };
        let jwk = match &signer {
            Signer::Jwk(jwk) => jwk.clone(),
            Signer::Account(index) => self.lock().accounts[*index].jwk.clone(),
        };
        let point = point(&jwk).ok_or_else(|| malformed("jwk"))?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| malformed("signature"))?;
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &point)
            .verify(format!("{protected}.{payload}").as_bytes(), &signature)
            .map_err(|_| self.problem(StatusCode::BAD_REQUEST, "malformed", "bad signature"))?;
        let payload = if payload.is_empty() {
            Value::Null
        } else {
            URL_SAFE_NO_PAD
                .decode(payload)
                .ok()
                .and_then(|p| serde_json::from_slice(&p).ok())
                .ok_or_else(|| malformed("payload"))?
        };
        Ok((payload, signer))
    }

    fn order_json(&self, order: &Order, id: &str) -> Value {
        let mut body = json!({
            "status": order.status,
            "identifiers": [{ "type": "dns", "value": order.domain }],
            "authorizations": order.authz.iter().map(|a| self.url(&format!("/authz/{a}"))).collect::<Vec<_>>(),
            "finalize": self.url(&format!("/order/{id}/finalize")),
        });
        if let Some(cert) = &order.certificate {
            body["certificate"] = self.url(&format!("/certificate/{cert}")).into();
        }
        if let Some(error) = &order.error {
            body["error"] = error.clone();
        }
        body
    }

    fn authz_json(&self, authz: &Authz, id: &str) -> Value {
        json!({
            "status": authz.status,
            "identifier": { "type": "dns", "value": authz.domain },
            "challenges": authz.challenges.iter().enumerate().map(|(i, c)| {
                let mut challenge = json!({
                    "type": c.kind,
                    "url": self.url(&format!("/challenge/{id}/{i}")),
                    "token": c.token,
                    "status": c.status,
                });
                if let Some(error) = &c.error {
                    challenge["error"] = error.clone();
                }
                challenge
            }).collect::<Vec<_>>(),
        })
    }

    /// Validates a challenge against the hub; the reason when it fails.
    async fn validate(
        &self,
        kind: &str,
        domain: &str,
        key_authorization: &str,
        token: &str,
    ) -> Result<(), String> {
        let (http_target, tls_target, fail) = {
            let inner = self.lock();
            (
                inner.http_target,
                inner.tls_target,
                inner.knobs.fail.contains(&kind),
            )
        };
        if fail {
            return Err("refused by the test".into());
        }
        match kind {
            "http-01" => {
                let target = http_target.ok_or("connection refused")?;
                let response = reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap()
                    .get(format!(
                        "http://{target}/.well-known/acme-challenge/{token}"
                    ))
                    .header(header::HOST, domain)
                    .send()
                    .await
                    .map_err(|e| format!("connection: {e}"))?;
                if response.status() != StatusCode::OK {
                    return Err(format!("status {}", response.status()));
                }
                let body = response.text().await.map_err(|e| e.to_string())?;
                (body.trim_end() == key_authorization)
                    .then_some(())
                    .ok_or_else(|| format!("wrong key authorization {body:?}"))
            }
            "tls-alpn-01" => {
                let target = tls_target.ok_or("connection refused")?;
                let mut config = ClientConfig::builder_with_provider(Arc::new(default_provider()))
                    .with_safe_default_protocol_versions()
                    .unwrap()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(AcceptAny))
                    .with_no_client_auth();
                config.alpn_protocols = vec![b"acme-tls/1".to_vec()];
                let tcp = TcpStream::connect(target)
                    .await
                    .map_err(|e| e.to_string())?;
                let tls = TlsConnector::from(Arc::new(config))
                    .connect(ServerName::try_from(domain.to_owned()).unwrap(), tcp)
                    .await
                    .map_err(|e| format!("TLS: {e}"))?;
                let (_, connection) = tls.get_ref();
                if connection.alpn_protocol() != Some(b"acme-tls/1") {
                    return Err("acme-tls/1 not negotiated".into());
                }
                let cert = connection
                    .peer_certificates()
                    .and_then(|c| c.first())
                    .ok_or("no certificate")?;
                let info = CertInfo::parse(cert).map_err(|e| e.to_string())?;
                let digest = Sha256::digest(key_authorization.as_bytes()).to_vec();
                if info.dns_names != [domain] {
                    return Err(format!("names {:?}", info.dns_names));
                }
                if info.acme_identifier != Some((digest, true)) {
                    return Err("wrong or non-critical acmeIdentifier".into());
                }
                info.signed_by(&info.spki).map_err(|e| e.to_string())
            }
            other => Err(format!("unknown type {other}")),
        }
    }
}

enum Signer {
    Jwk(Value),
    Account(usize),
}

fn point(jwk: &Value) -> Option<Vec<u8>> {
    if jwk["kty"] != "EC" || jwk["crv"] != "P-256" {
        return None;
    }
    let x = URL_SAFE_NO_PAD.decode(jwk["x"].as_str()?).ok()?;
    let y = URL_SAFE_NO_PAD.decode(jwk["y"].as_str()?).ok()?;
    let mut point = vec![4];
    point.extend(x);
    point.extend(y);
    (point.len() == 65).then_some(point)
}

fn thumbprint(jwk: &Value) -> String {
    jose::thumbprint_of(&[
        ("crv", "P-256"),
        ("kty", "EC"),
        ("x", jwk["x"].as_str().unwrap()),
        ("y", jwk["y"].as_str().unwrap()),
    ])
}

async fn directory(State(fake): State<FakeAcme>) -> Response {
    let mut body = json!({
        "newNonce": fake.url("/new-nonce"),
        "newAccount": fake.url("/new-account"),
        "newOrder": fake.url("/new-order"),
        "revokeCert": fake.url("/revoke"),
        "keyChange": fake.url("/key-change"),
        "meta": { "termsOfService": "https://ca.example.test/terms.pdf" },
    });
    if fake.lock().knobs.ari.is_some() {
        body["renewalInfo"] = fake.url("/renewal-info").into();
    }
    axum::Json(body).into_response()
}

async fn new_nonce(State(fake): State<FakeAcme>) -> Response {
    let mut response = StatusCode::OK.into_response();
    response.headers_mut().insert(
        "replay-nonce",
        HeaderValue::from_str(&fake.nonce()).unwrap(),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn new_account(State(fake): State<FakeAcme>, uri: Uri, body: Bytes) -> Response {
    let (payload, signer) = match fake.verify(&uri, &body, true) {
        Ok(verified) => verified,
        Err(response) => return response,
    };
    let Signer::Jwk(jwk) = signer else {
        unreachable!()
    };
    if payload["termsOfServiceAgreed"] != true {
        return fake.problem(
            StatusCode::FORBIDDEN,
            "userActionRequired",
            "agree to the terms",
        );
    }
    let contact: Vec<String> = payload["contact"]
        .as_array()
        .map(|c| {
            c.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let thumb = thumbprint(&jwk);
    let existing = {
        let inner = fake.lock();
        inner
            .accounts
            .iter()
            .find(|a| a.thumbprint == thumb)
            .map(|a| a.url.clone())
    };
    if let Some(url) = existing {
        // RFC 8555 § 7.3.1: the existing account, its old contact.
        let old = fake.lock().log.contacts.last().cloned().unwrap_or_default();
        return fake.reply(
            StatusCode::OK,
            Some(url),
            json!({ "status": "valid", "contact": old }),
        );
    }
    let url = fake.url(&format!("/account/{}", fake.id()));
    {
        let mut inner = fake.lock();
        inner.accounts.push(Account {
            url: url.clone(),
            jwk,
            thumbprint: thumb,
        });
        inner.log.accounts.push(url.clone());
        inner.log.contacts.push(contact.clone());
    }
    fake.reply(
        StatusCode::CREATED,
        Some(url),
        json!({ "status": "valid", "contact": contact }),
    )
}

async fn account(State(fake): State<FakeAcme>, uri: Uri, body: Bytes) -> Response {
    let (payload, _) = match fake.verify(&uri, &body, false) {
        Ok(verified) => verified,
        Err(response) => return response,
    };
    if let Some(contact) = payload["contact"].as_array() {
        let contact: Vec<String> = contact
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        fake.lock().log.contacts.push(contact);
    }
    fake.reply(StatusCode::OK, None, json!({ "status": "valid" }))
}

async fn new_order(State(fake): State<FakeAcme>, uri: Uri, body: Bytes) -> Response {
    let (payload, signer) = match fake.verify(&uri, &body, false) {
        Ok(verified) => verified,
        Err(response) => return response,
    };
    let Signer::Account(index) = signer else {
        unreachable!()
    };
    {
        let mut inner = fake.lock();
        if inner.knobs.rate_limited > 0 {
            inner.knobs.rate_limited -= 1;
            inner.log.rate_limits_sent += 1;
            let retry_after = inner.knobs.retry_after;
            drop(inner);
            let mut response = fake.problem(
                StatusCode::TOO_MANY_REQUESTS,
                "rateLimited",
                "too many new orders",
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after));
            return response;
        }
    }
    let identifiers = payload["identifiers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let [identifier] = identifiers.as_slice() else {
        return fake.problem(StatusCode::BAD_REQUEST, "rejectedIdentifier", "one name");
    };
    if identifier["type"] != "dns" {
        return fake.problem(StatusCode::BAD_REQUEST, "unsupportedIdentifier", "dns only");
    }
    let domain = identifier["value"].as_str().unwrap_or_default().to_owned();
    let replaces = payload["replaces"].as_str().map(str::to_owned);
    let (order_id, authz_id) = (fake.id(), fake.id());
    let offer = fake.lock().knobs.offer.clone();
    let order = Order {
        account: fake.lock().accounts[index].url.clone(),
        domain: domain.clone(),
        status: "pending".into(),
        authz: vec![authz_id.clone()],
        certificate: None,
        error: None,
        processing_left: 0,
    };
    let body = fake.order_json(&order, &order_id);
    {
        let mut inner = fake.lock();
        inner.log.orders.push(replaces);
        inner.authz.insert(
            authz_id,
            Authz {
                domain,
                status: "pending".into(),
                challenges: offer
                    .iter()
                    .map(|kind| Challenge {
                        kind: (*kind).to_owned(),
                        token: URL_SAFE_NO_PAD.encode(Sha256::digest(format!("{kind}{order_id}"))),
                        status: "pending".into(),
                        error: None,
                    })
                    .collect(),
            },
        );
        inner.orders.insert(order_id.clone(), order);
    }
    fake.reply(
        StatusCode::CREATED,
        Some(fake.url(&format!("/order/{order_id}"))),
        body,
    )
}

/// The order's status follows its authorizations.
fn refresh(inner: &mut Inner, id: &str) {
    let Some(order) = inner.orders.get(id) else {
        return;
    };
    if order.status != "pending" {
        return;
    }
    let states: Vec<String> = order
        .authz
        .iter()
        .map(|a| inner.authz[a].status.clone())
        .collect();
    let order = inner.orders.get_mut(id).unwrap();
    if states.iter().any(|s| s == "invalid") {
        order.status = "invalid".into();
    } else if states.iter().all(|s| s == "valid") {
        order.status = "ready".into();
    }
}

async fn order(
    State(fake): State<FakeAcme>,
    uri: Uri,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Err(response) = fake.verify(&uri, &body, false) {
        return response;
    }
    let (body, processing) = {
        let mut inner = fake.lock();
        refresh(&mut inner, &id);
        let Some(order) = inner.orders.get_mut(&id) else {
            drop(inner);
            return fake.problem(StatusCode::NOT_FOUND, "malformed", "no such order");
        };
        let processing = order.status == "processing";
        if processing {
            if order.processing_left == 0 {
                order.status = "valid".into();
            } else {
                order.processing_left -= 1;
            }
        }
        let order = &inner.orders[&id];
        (
            fake.order_json(order, &id),
            processing && order.status == "processing",
        )
    };
    let mut response = fake.reply(StatusCode::OK, None, body);
    if processing {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("0"));
    }
    response
}

async fn authz(
    State(fake): State<FakeAcme>,
    uri: Uri,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Err(response) = fake.verify(&uri, &body, false) {
        return response;
    }
    let body = {
        let inner = fake.lock();
        let Some(authz) = inner.authz.get(&id) else {
            drop(inner);
            return fake.problem(StatusCode::NOT_FOUND, "malformed", "no such authorization");
        };
        fake.authz_json(authz, &id)
    };
    fake.reply(StatusCode::OK, None, body)
}

async fn challenge(
    State(fake): State<FakeAcme>,
    uri: Uri,
    Path((id, index)): Path<(String, usize)>,
    body: Bytes,
) -> Response {
    let (_, signer) = match fake.verify(&uri, &body, false) {
        Ok(verified) => verified,
        Err(response) => return response,
    };
    let Signer::Account(account) = signer else {
        unreachable!()
    };
    let (kind, token, domain, thumb) = {
        let inner = fake.lock();
        let authz = &inner.authz[&id];
        let challenge = &authz.challenges[index];
        (
            challenge.kind.clone(),
            challenge.token.clone(),
            authz.domain.clone(),
            inner.accounts[account].thumbprint.clone(),
        )
    };
    let key_authorization = format!("{token}.{thumb}");
    let result = fake
        .validate(&kind, &domain, &key_authorization, &token)
        .await;
    let body = {
        let mut inner = fake.lock();
        inner.log.validations.push((kind.clone(), result.is_ok()));
        let authz = inner.authz.get_mut(&id).unwrap();
        let challenge = &mut authz.challenges[index];
        match result {
            Ok(()) => {
                challenge.status = "valid".into();
                authz.status = "valid".into();
            }
            Err(reason) => {
                challenge.status = "invalid".into();
                challenge.error = Some(json!({
                    "type": "urn:ietf:params:acme:error:connection",
                    "detail": reason,
                }));
                authz.status = "invalid".into();
            }
        }
        let authz = &inner.authz[&id];
        let mut body = fake.authz_json(authz, &id)["challenges"][index].clone();
        body["status"] = authz.challenges[index].status.clone().into();
        body
    };
    fake.reply(StatusCode::OK, None, body)
}

async fn finalize(
    State(fake): State<FakeAcme>,
    uri: Uri,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let (payload, _) = match fake.verify(&uri, &body, false) {
        Ok(verified) => verified,
        Err(response) => return response,
    };
    let refused = fake.lock().knobs.finalize_error.take();
    if let Some(kind) = refused {
        return fake.problem(StatusCode::FORBIDDEN, kind, "finalize refused for the test");
    }
    let csr = payload["csr"]
        .as_str()
        .and_then(|csr| URL_SAFE_NO_PAD.decode(csr).ok())
        .and_then(|der| CsrInfo::parse(&der).ok());
    let Some(csr) = csr else {
        return fake.problem(StatusCode::BAD_REQUEST, "badCSR", "CSR not parsable");
    };
    let (domain, ready) = {
        let mut inner = fake.lock();
        refresh(&mut inner, &id);
        let order = &inner.orders[&id];
        (order.domain.clone(), order.status == "ready")
    };
    if !ready {
        return fake.problem(StatusCode::FORBIDDEN, "orderNotReady", "not ready");
    }
    if csr.dns_names != [domain.as_str()] {
        return fake.problem(
            StatusCode::BAD_REQUEST,
            "badCSR",
            "names do not match the order",
        );
    }
    let (backdate, lifetime, processing) = {
        let inner = fake.lock();
        (
            inner.knobs.backdate,
            inner.knobs.lifetime,
            inner.knobs.processing_polls,
        )
    };
    let now = Timestamp::now();
    let serial = fake.id();
    let leaf = CertificateParams {
        serial: serial.as_bytes().to_vec(),
        issuer: x509::name(Some("Fake ACME CA")),
        subject: x509::name(None),
        not_before: now - backdate,
        not_after: now - backdate + lifetime,
        spki: csr.spki,
        extensions: vec![
            Extension::dns_names(&[&domain]),
            Extension::server_auth(),
            Extension::authority_key_id(&CA_KEY_ID),
        ],
    }
    .sign(&fake.ca_key)
    .unwrap();
    let pem = format!(
        "{}{}",
        pem("CERTIFICATE", &leaf),
        pem("CERTIFICATE", &fake.ca_cert)
    );
    let body = {
        let mut inner = fake.lock();
        inner.log.issued += 1;
        let cert_id = serial;
        inner.certificates.insert(cert_id.clone(), pem);
        let order = inner.orders.get_mut(&id).unwrap();
        order.status = "processing".into();
        order.processing_left = processing;
        order.certificate = Some(cert_id);
        let order = &inner.orders[&id];
        fake.order_json(order, &id)
    };
    let mut response = fake.reply(StatusCode::OK, None, body);
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("0"));
    response
}

async fn certificate(
    State(fake): State<FakeAcme>,
    uri: Uri,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Err(response) = fake.verify(&uri, &body, false) {
        return response;
    }
    let pem = fake.lock().certificates.get(&id).cloned();
    let Some(pem) = pem else {
        return fake.problem(StatusCode::NOT_FOUND, "malformed", "no such certificate");
    };
    let mut response = (StatusCode::OK, pem).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/pem-certificate-chain"),
    );
    response.headers_mut().insert(
        "replay-nonce",
        HeaderValue::from_str(&fake.nonce()).unwrap(),
    );
    response
}

async fn renewal_info(State(fake): State<FakeAcme>, Path(id): Path<String>) -> Response {
    let mut inner = fake.lock();
    inner.log.ari_requests += 1;
    let Some((start, end)) = inner.knobs.ari else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // The ID must be the AKI and a serial of a certificate issued here.
    let aki = URL_SAFE_NO_PAD.encode(CA_KEY_ID);
    if !id.starts_with(&format!("{aki}.")) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let now = Timestamp::now();
    let mut headers = HeaderMap::new();
    headers.insert(header::RETRY_AFTER, HeaderValue::from_static("21600"));
    (
        headers,
        axum::Json(json!({
            "suggestedWindow": {
                "start": (now + start).to_string(),
                "end": (now + end).to_string(),
            }
        })),
    )
        .into_response()
}

pub fn pem(label: &str, der: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    let body = STANDARD.encode(der);
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    format!(
        "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
        lines.join("\n")
    )
}

/// The validation reads the challenge certificate, whatever it is.
#[derive(Debug)]
struct AcceptAny;

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_p256(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_p256(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

/// The handshake signature with the challenge certificate's P-256 key.
/// rustls' own helpers parse the certificate with webpki, which refuses the
/// critical `acmeIdentifier`; a validator reads it with its own parser.
fn verify_p256(
    message: &[u8],
    cert: &CertificateDer<'_>,
    dss: &DigitallySignedStruct,
) -> Result<HandshakeSignatureValid, rustls::Error> {
    let fail = || rustls::Error::General("bad handshake signature".into());
    if dss.scheme != SignatureScheme::ECDSA_NISTP256_SHA256 {
        return Err(fail());
    }
    let info = CertInfo::parse(cert).map_err(|_| fail())?;
    let point = x509::p256_point(&info.spki).map_err(|_| fail())?;
    UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, &point)
        .verify(message, dss.signature())
        .map_err(|_| fail())?;
    Ok(HandshakeSignatureValid::assertion())
}

/// How long the tests wait for something at most.
pub const PATIENCE: Duration = Duration::from_secs(20);
