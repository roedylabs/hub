//! Mock of the JMAP endpoints a sending account uses (behaviour as in
//! Stalwart v1), on `127.0.0.1` with a random port: session via
//! `/.well-known/jmap`, `Identity/get`, `Mailbox/get`, `Email/set` and
//! `EmailSubmission/set`. With [`MAIL_USER`]/[`MAIL_PASSWORD`] or
//! [`MAIL_TOKEN`] an account with two identities; anything else is rejected
//! (counted in [`MailState::failed_logins`]).
//!
//! Taken over from Roedy Mail (`crates/server/tests/fake_stalwart`) and
//! trimmed to sending: no OAuth, registration, CORS, quota or throttling.

#![allow(dead_code, reason = "not every test file uses everything")]

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::oneshot};

/// Sending account with app password or token (tests only).
pub const MAIL_USER: &str = "noreply@example.com";
pub const MAIL_PASSWORD: &str = "app-password-1";
pub const MAIL_TOKEN: &str = "token-1";
const ACCOUNT_ID: &str = "a1";

/// What the server can do.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// `EmailSubmission/set` rejects with this error type.
    pub submission_error: Option<&'static str>,
    /// Account without a "Sent" mailbox.
    pub without_sent: bool,
}

#[derive(Clone)]
struct Fake {
    base: String,
    options: Options,
    mail: Arc<Mutex<MailState>>,
}

/// What happened in the account.
#[derive(Debug, Default, Clone)]
pub struct MailState {
    /// Mails in the account by ID.
    pub emails: BTreeMap<String, Value>,
    /// Sent mails (state at sending).
    pub submitted: Vec<Value>,
    /// Rejected sign-ins.
    pub failed_logins: usize,
    /// Signed-in requests to the `apiUrl`.
    pub requests: usize,
    /// `EmailSubmission/set` rejects mails to this address
    /// ([`Server::reject_mail_to`]).
    reject_to: Option<String>,
    /// Mails rejected that way (state at the attempt).
    pub rejected: Vec<Value>,
    next_id: u32,
}

impl MailState {
    /// Sent mails to `to`.
    pub fn sent_to(&self, to: &str) -> Vec<&Value> {
        self.submitted
            .iter()
            .filter(|email| email["to"][0]["email"] == to)
            .collect()
    }

    /// Text part of a sent mail.
    pub fn text(email: &Value) -> String {
        email["bodyValues"]["text"]["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
}

pub struct Server {
    pub url: String,
    mail: Arc<Mutex<MailState>>,
    stop: Mutex<Option<oneshot::Sender<()>>>,
}

impl Server {
    pub async fn start(options: Options) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let mail = Arc::new(Mutex::new(MailState::default()));
        let fake = Fake {
            base: url.clone(),
            options,
            mail: mail.clone(),
        };
        let app = Router::new()
            .route("/.well-known/jmap", get(well_known))
            .route("/jmap/session", get(session))
            .route("/jmap/", post(api))
            .with_state(fake);
        let (stop, stopped) = oneshot::channel();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    stopped.await.ok();
                })
                .await
                .unwrap();
        });
        Self {
            url,
            mail,
            stop: Mutex::new(Some(stop)),
        }
    }

    /// Stops the server; afterwards it is no longer reachable.
    pub async fn stop(&self) {
        if let Some(stop) = self.stop.lock().unwrap().take() {
            stop.send(()).ok();
        }
        while tokio::net::TcpStream::connect(self.url.trim_start_matches("http://"))
            .await
            .is_ok()
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    pub fn mail(&self) -> MailState {
        self.mail.lock().unwrap().clone()
    }

    /// From now on rejects mails to `to` (`None`: send all again).
    pub fn reject_mail_to(&self, to: Option<&str>) {
        self.mail.lock().unwrap().reject_to = to.map(str::to_owned);
    }
}

/// Address where nobody answers: without root nobody can bind port 1.
pub fn unreachable_url() -> String {
    "http://127.0.0.1:1".into()
}

enum Login {
    Anonymous,
    Valid,
    Invalid,
}

fn login(fake: &Fake, headers: &HeaderMap) -> Login {
    use base64::Engine as _;
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Login::Anonymous;
    };
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{MAIL_USER}:{MAIL_PASSWORD}"))
    );
    if value == basic.as_str() || value == format!("Bearer {MAIL_TOKEN}").as_str() {
        Login::Valid
    } else {
        fake.mail.lock().unwrap().failed_logins += 1;
        Login::Invalid
    }
}

async fn well_known() -> Response {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(header::LOCATION, "/jmap/session")],
    )
        .into_response()
}

async fn session(State(fake): State<Fake>, headers: HeaderMap) -> Response {
    let (accounts, primary, username) = match login(&fake, &headers) {
        Login::Invalid => return StatusCode::UNAUTHORIZED.into_response(),
        Login::Valid => (
            json!({ ACCOUNT_ID: { "name": MAIL_USER, "isPersonal": true } }),
            json!({
                "urn:ietf:params:jmap:mail": ACCOUNT_ID,
                "urn:ietf:params:jmap:submission": ACCOUNT_ID,
            }),
            MAIL_USER,
        ),
        Login::Anonymous => (json!({}), json!({}), ""),
    };
    let base = &fake.base;
    Json(json!({
        "capabilities": {
            "urn:ietf:params:jmap:core": { "maxSizeUpload": 50_000_000 },
            "urn:ietf:params:jmap:mail": {},
            "urn:ietf:params:jmap:submission": {},
        },
        "accounts": accounts,
        "primaryAccounts": primary,
        "username": username,
        "apiUrl": format!("{base}/jmap/"),
        "downloadUrl": format!("{base}/jmap/download/{{accountId}}/{{blobId}}/{{name}}?accept={{type}}"),
        "uploadUrl": format!("{base}/jmap/upload/{{accountId}}/"),
        "eventSourceUrl": format!("{base}/jmap/eventsource/?types={{types}}&closeafter={{closeafter}}&ping={{ping}}"),
        "state": "0",
    }))
    .into_response()
}

/// Two identities of the test account.
fn identities() -> Value {
    json!([
        { "id": "i1", "name": "Example Ltd", "email": MAIL_USER, "mayDelete": false },
        { "id": "i2", "name": "Example Support", "email": "support@example.com", "mayDelete": true },
    ])
}

/// `POST /jmap/`: method calls one after another, creation IDs (`#m`) apply
/// in the whole request (RFC 8620, 5.3).
async fn api(State(fake): State<Fake>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let Login::Valid = login(&fake, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let calls = body["methodCalls"].as_array().cloned().unwrap_or_default();
    let mut state = fake.mail.lock().unwrap();
    state.requests += 1;
    let mut created = HashMap::new();
    let mut responses = Vec::new();
    for call in calls {
        let (name, args, id) = (call[0].as_str().unwrap_or_default(), &call[1], &call[2]);
        if args["accountId"] != ACCOUNT_ID {
            responses.push(json!(["error", { "type": "accountNotFound" }, id]));
            continue;
        }
        match name {
            "Identity/get" => responses.push(json!([
                name,
                { "accountId": ACCOUNT_ID, "state": "1", "list": identities(), "notFound": [] },
                id,
            ])),
            "Mailbox/get" => {
                let mut list = vec![
                    json!({ "id": "inbox", "role": "inbox" }),
                    json!({ "id": "drafts", "role": "drafts" }),
                ];
                if !fake.options.without_sent {
                    list.push(json!({ "id": "sent", "role": "sent" }));
                }
                responses.push(json!([
                    name,
                    { "accountId": ACCOUNT_ID, "state": "1", "list": list, "notFound": [] },
                    id,
                ]));
            }
            "Email/set" => {
                let result = email_set(&mut state, args, &mut created);
                responses.push(json!([name, result, id]));
            }
            "EmailSubmission/set" => {
                for response in submission_set(&fake.options, &mut state, args, &created) {
                    responses.push(json!([response.0, response.1, id]));
                }
            }
            _ => responses.push(json!(["error", { "type": "unknownMethod" }, id])),
        }
    }
    Json(json!({ "methodResponses": responses, "sessionState": "1" })).into_response()
}

fn email_set(
    state: &mut MailState,
    args: &Value,
    created_ids: &mut HashMap<String, String>,
) -> Value {
    let mut created = serde_json::Map::new();
    let mut not_created = serde_json::Map::new();
    for (key, email) in args["create"].as_object().cloned().unwrap_or_default() {
        let valid = email["mailboxIds"]
            .as_object()
            .is_some_and(|m| !m.is_empty())
            && email["bodyValues"].is_object();
        if !valid {
            not_created.insert(key, json!({ "type": "invalidProperties" }));
            continue;
        }
        state.next_id += 1;
        let id = format!("e{}", state.next_id);
        state.emails.insert(id.clone(), email);
        created.insert(key.clone(), json!({ "id": id, "blobId": format!("b{id}"), "threadId": format!("t{id}"), "size": 1000 }));
        created_ids.insert(key, id);
    }
    let mut updated = serde_json::Map::new();
    for (id, patch) in args["update"].as_object().cloned().unwrap_or_default() {
        if let Some(email) = state.emails.get_mut(&id) {
            apply_patch(email, &patch);
            updated.insert(id, Value::Null);
        }
    }
    let mut destroyed = Vec::new();
    for id in args["destroy"].as_array().cloned().unwrap_or_default() {
        if let Some(id) = id.as_str()
            && state.emails.remove(id).is_some()
        {
            destroyed.push(json!(id));
        }
    }
    json!({
        "accountId": ACCOUNT_ID,
        "newState": "2",
        "created": created,
        "notCreated": not_created,
        "updated": updated,
        "destroyed": destroyed,
    })
}

/// Patch per RFC 8620, 5.3 (`mailboxIds/<id>`, `keywords/<k>`).
fn apply_patch(email: &mut Value, patch: &Value) {
    for (path, value) in patch.as_object().cloned().unwrap_or_default() {
        let (property, key) = path.split_once('/').unwrap();
        let map = email[property].as_object_mut().unwrap();
        if value.is_null() {
            map.remove(key);
        } else {
            map.insert(key.to_owned(), value);
        }
    }
}

/// Like Stalwart: the identity must exist, `From` must match it; after
/// sending the implicit `Email/set` (RFC 8621, 7.5) with the same ID.
fn submission_set(
    options: &Options,
    state: &mut MailState,
    args: &Value,
    created_emails: &HashMap<String, String>,
) -> Vec<(&'static str, Value)> {
    let mut created = serde_json::Map::new();
    let mut not_created = serde_json::Map::new();
    let mut sent = HashMap::new();
    for (key, submission) in args["create"].as_object().cloned().unwrap_or_default() {
        let email_id = submission["emailId"].as_str().unwrap_or_default();
        let email_id = match email_id.strip_prefix('#') {
            Some(reference) => created_emails.get(reference).cloned(),
            None => Some(email_id.to_owned()),
        };
        let identity = identities()
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == submission["identityId"])
            .cloned();
        let (Some(email_id), Some(identity)) = (email_id, identity) else {
            not_created.insert(key, json!({ "type": "invalidProperties" }));
            continue;
        };
        let Some(email) = state.emails.get(&email_id).cloned() else {
            not_created.insert(
                key,
                json!({ "type": "invalidProperties", "description": "Email not found." }),
            );
            continue;
        };
        if email["from"][0]["email"] != identity["email"] {
            not_created.insert(key, json!({ "type": "forbiddenFrom" }));
            continue;
        }
        if let Some(kind) = options.submission_error {
            not_created.insert(key, json!({ "type": kind, "description": "Rejected." }));
            continue;
        }
        if state.reject_to.is_some()
            && email["to"][0]["email"].as_str() == state.reject_to.as_deref()
        {
            not_created.insert(
                key,
                json!({ "type": "forbiddenToSend", "description": "Rejected." }),
            );
            state.rejected.push(email);
            continue;
        }
        state.submitted.push(email);
        created.insert(
            key.clone(),
            json!({ "id": format!("s{email_id}"), "undoStatus": "final" }),
        );
        sent.insert(format!("#{key}"), email_id);
    }
    let mut responses = vec![(
        "EmailSubmission/set",
        json!({ "accountId": ACCOUNT_ID, "newState": "2", "created": created, "notCreated": not_created }),
    )];
    let mut update = serde_json::Map::new();
    for (reference, patch) in args["onSuccessUpdateEmail"]
        .as_object()
        .cloned()
        .unwrap_or_default()
    {
        if let Some(id) = sent.get(&reference) {
            update.insert(id.clone(), patch);
        }
    }
    let destroy: Vec<Value> = args["onSuccessDestroyEmail"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|reference| sent.get(reference.as_str()?))
        .map(|id| json!(id))
        .collect();
    if !update.is_empty() || !destroy.is_empty() {
        let args = json!({ "accountId": ACCOUNT_ID, "update": update, "destroy": destroy });
        responses.push(("Email/set", email_set(state, &args, &mut HashMap::new())));
    }
    responses
}
