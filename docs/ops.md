# Operating Roedy Hub

> Configuration, TLS and health checks of the server. Installation with Docker Compose, the data directory, backups
> and troubleshooting follow with the delivery milestone ([`plan.md`](plan.md)).

Roedy Hub is a single process with an embedded web app and an SQLite database. It terminates TLS itself and gets its
certificate from an ACME CA such as Let's Encrypt ([TLS](#tls)); no proxy is needed in front. It listens on the
**public** port (HTTPS), on a plain **HTTP** port that only answers ACME challenges and redirects to HTTPS, and on the
**internal** port, which only the apps reach over the private network
([app contract › Internal API](app-contract.md#7-internal-api)).

```
Browser ──HTTPS──► roedy-hub :8443 (public, published as 443) ──► /data (SQLite, key, certificate)
        ──HTTP───► roedy-hub :8080 (ACME HTTP-01 and redirect, published as 80)
                     apps (private network) ──HTTP──► roedy-hub :8081 (internal, never published)
```

With `ROEDY_TLS=off` (the default) the public port speaks plain HTTP on `:8080` instead, for development or behind a
TLS proxy of your own.

## Configuration

All settings come from environment variables. Values are read as text without surrounding whitespace, so a key or
path made only of digits works as well. Empty values count as unset (except for `ROEDY_INTERNAL_LISTEN`, see below),
unknown `ROEDY_*` variables are ignored, invalid values prevent the start with a line `error: …` that names the
variable (such as `error: Invalid configuration – ROEDY_LISTEN: …`). The names and meanings are the same as in
Roedy Mail.

| Variable | Default | Meaning |
|---|---|---|
| `ROEDY_PUBLIC_URL` | – | **Required.** Public address of the hub, e.g. `https://app.example.com`: scheme, host and optionally a port, without path, credentials, query or fragment. The hub owns the root of its origin; the apps live under `/<slug>/`. Without it, `serve` exits with `error: ROEDY_PUBLIC_URL is not set – …` before it creates anything. |
| `ROEDY_LISTEN` | `0.0.0.0:8080`, with TLS on `0.0.0.0:8443` | Address and port of the public server: web app, `/api/v1`, `/healthz`, `/readyz` and the apps under `/<slug>/` ([Gateway](#gateway)). HTTPS when `ROEDY_TLS` is on, plain HTTP otherwise. |
| `ROEDY_INTERNAL_LISTEN` | `0.0.0.0:8081` | Address and port of the internal HTTP server for the apps (`/internal/v1/…`). **Never publish this port** and never forward it through the reverse proxy. Set to empty to turn it off; then no app can pair. It must not overlap with `ROEDY_LISTEN`. Apps pair, send their manifest and read users and keys here (docs/app-contract.md › Internal API); every other path answers `404`. |
| `ROEDY_DATA_DIR` | `data` relative to the working directory | Directory with the database `roedy.sqlite3` and the key `secret.key`; created with `0700`, files `0600`. Must be on a local file system (SQLite in WAL mode). |
| `ROEDY_LOG` | `info` | Log filter in the format of [`EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html), e.g. `warn` or `info,roedy_hub_server=debug`. `info,tower_http=debug` logs every request (method and path, tokens in paths replaced by `***`, never the query). The setup token always appears. |
| `ROEDY_LOG_FORMAT` | `json` | `json` (one line per event) or `text`. Logs go to stderr. |
| `ROEDY_WEB_DIR` | – | Directory with the web app build (e.g. `apps/web/build`). Takes precedence over the embedded web app; for development. |
| `ROEDY_TRUST_PROXY` | `false` | Reverse proxies in front of the server: `true` for one, otherwise their number (at most 10). The client IP is then the entry in `X-Forwarded-For` appended by the front-most proxy, and the server sends HSTS. Only set it when the public port is reachable exclusively via the proxy and every proxy appends to `X-Forwarded-For` instead of passing it through. **Leave it off when the hub is exposed directly** (built-in TLS): otherwise any client could choose its IP address with `X-Forwarded-For`. The apps receive the trusted entries plus the hub's own peer; set their `ROEDY_TRUST_PROXY` to this value plus one. |
| `ROEDY_SECRET_KEY` | – | Instance key for secrets in the database: 32 bytes as 64 hex characters (`openssl rand -hex 32`) or Base64 (`openssl rand -base64 32`). If it is missing as well as `ROEDY_SECRET_KEY_FILE`, the first start creates `<ROEDY_DATA_DIR>/secret.key` (`0600`) and uses it afterwards. Without the key the encrypted values can no longer be read; keep a copy outside the data directory. The keys that sign the identities for the apps are encrypted with it as well: with another instance key the hub replaces them at the next start, and paired apps fetch the new one on their own. |
| `ROEDY_SECRET_KEY_FILE` | – | File with this key, e.g. a Docker secret (`/run/secrets/roedy_secret_key`). Not together with `ROEDY_SECRET_KEY`. |
| `ROEDY_TLS` | `off` | Built-in TLS ([TLS](#tls)): `off` (plain HTTP on `ROEDY_LISTEN`), `acme` (certificates from an ACME CA) or `files` (a certificate from files). With `acme` or `files`, `ROEDY_PUBLIC_URL` must start with `https://`; the certificate is for its host. |
| `ROEDY_HTTP_LISTEN` | `0.0.0.0:8080` | With TLS on: address and port of the plain HTTP server, which only answers ACME HTTP-01 challenges and redirects everything else (`308`) to the same path under `ROEDY_PUBLIC_URL` – no web app, no API. Unused with `ROEDY_TLS=off`. Must not overlap with `ROEDY_LISTEN` or `ROEDY_INTERNAL_LISTEN`. |
| `ROEDY_ACME_DIRECTORY` | `https://acme-v02.api.letsencrypt.org/directory` | `acme`: the CA's directory. Let's Encrypt's staging directory (`https://acme-staging-v02.api.letsencrypt.org/directory`) is for trying a setup out; its certificates are not trusted by browsers. `http://` only for a test CA on loopback. Changing it gets a new certificate from the new CA at the next start. |
| `ROEDY_ACME_EMAIL` | – | `acme`: contact address for the account at the CA (optional; Let's Encrypt sends no expiry mails anymore). |
| `ROEDY_ACME_AGREE_TOS` | `false` | `acme`: **required** `true` – you accept the CA's terms of service (Let's Encrypt: the Subscriber Agreement linked from its directory). Without it, `serve` refuses to start and says so. |
| `ROEDY_TLS_CERT_FILE` | – | `files`: the certificate chain in PEM, leaf first. Reloaded within 10 seconds when it changes, without a restart; a replacement that cannot be used is logged and the previous certificate stays. |
| `ROEDY_TLS_KEY_FILE` | – | `files`: its private key in PEM (PKCS#8, SEC1 or PKCS#1; ECDSA, Ed25519 or RSA). |

**Initial setup:** as long as no administrator exists, every start writes a setup token to the log
(`Setup-Token: XXXX-XXXX-XXXX`). `roedy-hub setup-token` shows it again.

**Commands:** `roedy-hub serve` starts the server, `roedy-hub healthcheck` checks it ([Health](#health)),
`roedy-hub migrate` applies pending database migrations (`serve` does that as well) and `roedy-hub setup-token` shows
the setup token. All of them read the same variables. On `SIGTERM` or `SIGINT` the server stops accepting connections
on all ports, closes the open streams and WebSockets to the apps and gives open requests 8 seconds to finish.

**Lost administrator access:** `roedy-hub reset-admin` restores an administrator's access – for example when the only
administrator lost both password and authenticator app. It sets a new random password and prints it, removes the
authenticator apps and recovery codes, signs out every device, discards open links from mails and unlocks the
account; the audit log records it. With several administrators, choose one with `--user <email address or username>`.
`--link` prints a link to set a new password (valid for an hour, needs `ROEDY_PUBLIC_URL`) instead of a password, so the
operator never learns it; the old password stops working right away. Run it next to the server with the same
`ROEDY_DATA_DIR`, for example in its container.

**Cleanup:** the server deletes expired sessions and links from mails every hour, audit log entries older than the
retention chosen under Administration › Audit log (30, 90 or 365 days, default 90) and events for the apps older
than 30 days ([Events](#events)) once a day.

## TLS

Roedy Hub terminates TLS itself (TLS 1.3 and 1.2, AEAD cipher suites with forward secrecy only, ALPN `http/1.1`) and,
with `ROEDY_TLS=acme`, gets and renews its certificate on its own with its built-in ACME client (RFC 8555). It answers
only TLS connections for the host of `ROEDY_PUBLIC_URL` (SNI); other names get no certificate. Responses carry
`Strict-Transport-Security: max-age=31536000` (without `includeSubDomains` and `preload`; submit the domain to the
preload list yourself if you want that). HTTP/2 is not offered: the gateway's WebSockets use HTTP/1.1's `Upgrade`.

**Ports and firewall.** In the container the hub runs as an unprivileged user and listens on `8443` (HTTPS) and `8080`
(HTTP); publish them as `443` and `80`, e.g. `ports: ["443:8443", "80:8080"]` in Compose. Open both in the firewall.
Port 80 is optional: the hub validates with TLS-ALPN-01 on port 443 first and falls back to HTTP-01 on port 80 only if
that fails; without port 80 there is no redirect from `http://` either. Never publish the internal port `8081`.

**DNS.** Before the first start, the host of `ROEDY_PUBLIC_URL` must resolve (`A`, and `AAAA` if the server has IPv6)
to the server, and ports 443 (and 80) must reach the hub from the internet: the CA connects to them to validate. A
CAA record, if you have one, must allow the CA (`letsencrypt.org`).

**Terms of service.** `ROEDY_TLS=acme` needs `ROEDY_ACME_AGREE_TOS=true`: by setting it you accept the CA's terms of
service. The hub logs their address when it registers the account.

**First start.** The hub starts at once and gets the certificate in the background, usually within a minute.
Until then the HTTPS port answers only the CA's validation, and `roedy-hub healthcheck` fails (give the container's
health check a `start_period` of a few minutes). If it fails, the hub logs why (`TLS certificate not obtained`) and
tries again after 1 minute, 5 minutes, 15 minutes, 1 hour and then every 3 hours – or later, if the CA asks for it
(rate limits). The time of the next attempt is stored, so restarting does not try earlier.

**Renewal.** A daily check renews the certificate 30 days before it expires (after two thirds of the lifetime for
shorter certificates) or earlier when the CA's renewal information (ARI) suggests it, and swaps it in without dropping
open connections; a failed renewal is tried again with the next daily check (at the earliest when the wait above is
over), long before the certificate expires. Issuance, renewal and the first failure of a series appear in
Administration › Audit log (category System).

**Storage.** The account key, the certificate's private key and the chain are kept in the database, the keys encrypted
with the instance key. With another instance key they cannot be decrypted: the hub then creates a new account and gets
a new certificate. Back the database up with the rest; there are no certificate files to keep.

**Trying it out.** Use Let's Encrypt's staging directory (`ROEDY_ACME_DIRECTORY`, see above) for a first test: it has
far higher rate limits, and its certificates are not trusted by browsers. Remove it (and restart) once everything
works; the hub then gets a certificate from production.

**Certificate from files.** With `ROEDY_TLS=files` the hub serves the certificate in `ROEDY_TLS_CERT_FILE` and
`ROEDY_TLS_KEY_FILE`, e.g. from your own CA or another ACME client, and reloads it when the files change. It must
contain the host of `ROEDY_PUBLIC_URL` (a wildcard covers one label); the start fails otherwise. The HTTP port only
redirects then.

**Behind your own proxy.** Keep `ROEDY_TLS=off`, let the proxy terminate TLS and forward to `ROEDY_LISTEN`, and set
`ROEDY_TRUST_PROXY`.

## Gateway

Every path whose first segment is an app's slug goes to that app's internal address, path and query unchanged
([app contract › Routing](app-contract.md#3-routing)); the hub's own paths and every reserved name stay with the hub.
The hub speaks HTTP/1.1 to the apps (TLS for an `https` internal address, verified against the system's roots), keeps
idle connections for a minute and passes bodies through as they arrive, in both directions, without a size or total
time limit. It answers itself with JSON `{"error": "<code>"}` – for page loads with the web app showing the error –
when:

| Status | `error` | When |
|---|---|---|
| `404` | `not_found` | No app with this slug, or `/<slug>/_roedy/…` from outside |
| `403` | `no_app_access` | Signed in, but without access to the app |
| `403` | `origin_mismatch` | A WebSocket upgrade from another origin |
| `431` | `headers_too_large` | More than 64 KiB of request headers; always JSON, also for page loads |
| `502` | `app_unreachable` | No connection to the app within 5 seconds (name lookup, TCP and TLS), or the app breaks it off before its response |
| `503` | `app_not_ready` | The app waits for pairing and the user is no hub administrator |
| `504` | `app_timeout` | No response headers from the app within 300 seconds after the hub took the request body's last part to send it (an upload has no time limit; an app that stops reading its last bytes counts as not answering) |

Every `502` and `504` is logged as a warning with the slug and the request's `X-Roedy-Request-Id`, which the app
receives as well. A session is checked at most every 10 seconds per cookie; signing out, revoking a device, locking or
removing a user and changing the password take effect at once and close the session's open streams and WebSockets.
Granting, changing or removing a user's access to an app (Administration › Users or › Apps) applies to their next
request; removing it also closes their open streams and WebSockets to that app.
Sessions that end otherwise (expired, signed out by `roedy-hub reset-admin`) lose them within 5 seconds, as long as
the database answers within a second; a busier database delays it until a check gets through.

## Events

Changes to users, sessions and access reach the apps as signed events
([app contract › Events](app-contract.md#10-events)), written in the same database transaction as the change and sent
right after it to `POST <internal address>/<slug>/_roedy/v1/events`, one at a time in order per app. An app that does
not answer `204` within 10 seconds gets the same event again after 10 s, 30 s, 1 min, 5 min, 15 min and then every
hour; the first failure of a series is logged as a warning (`event delivery failed, retrying`, with the slug, `seq`
and the reason: `status <code>`, `timeout` or `not reachable`), the repetitions only at `debug`. After 7 days the hub
gives up (`event delivery given up until the app is back`) and tries again as soon as the app sends its manifest, reads
missed events, pairs again or answers its health check. Events written by `roedy-hub reset-admin` go out within a
minute. Events are kept for 30 days for apps that catch up; removing an app deletes its events.

## Health

| Endpoint | Response |
|---|---|
| `GET /healthz` | `200 {"status":"ok"}` as long as the process responds |
| `GET /readyz` | `200 {"status":"ok"}` if the database is readable within 2 seconds, otherwise `503 {"status":"unavailable"}` and a warning in the log |

Both are on the public port only, need no sign-in and reveal nothing but the health (`Cache-Control: no-store`). For
external monitoring, `/readyz` via the public address is the right choice:

```sh
curl -fsS https://app.example.com/readyz
```

`roedy-hub healthcheck` queries `/healthz` without curl (the image has none) and exits with `0` if it answers `200`
within 3 seconds. It reads `ROEDY_LISTEN` and connects to its port over loopback (`127.0.0.1`, for `[::]` first
`::1`), or to the address itself if `ROEDY_LISTEN` names one. With `ROEDY_TLS` on it speaks TLS with the host of
`ROEDY_PUBLIC_URL` as server name and does not check the certificate (it reaches its own process); before the first
certificate exists it fails. Use it as the container's health check:

```yaml
healthcheck:
  test: ["CMD", "roedy-hub", "healthcheck"]
  interval: 30s
  timeout: 5s
  start_period: 3m
  retries: 3
```
