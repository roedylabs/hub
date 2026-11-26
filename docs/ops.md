# Operating Roedy Hub

> Configuration and health checks of the server. Installation with Docker Compose, the reverse proxy, the data
> directory, backups and troubleshooting follow with the delivery milestone ([`plan.md`](plan.md)).

Roedy Hub is a single process with an embedded web app and an SQLite database. It listens on two ports: the
**public** one, which the reverse proxy forwards to, and the **internal** one, which only the apps reach over the
private network ([app contract › Internal API](app-contract.md#7-internal-api)).

```
Browser ──HTTPS──► reverse proxy ──HTTP──► roedy-hub :8080 (public)   ──► /data (SQLite, key)
                     apps (private network) ──HTTP──► roedy-hub :8081 (internal, never published)
```

## Configuration

All settings come from environment variables. Values are read as text without surrounding whitespace, so a key or
path made only of digits works as well. Empty values count as unset (except for `ROEDY_INTERNAL_LISTEN`, see below),
unknown `ROEDY_*` variables are ignored, invalid values prevent the start with a line `error: …` that names the
variable (such as `error: Invalid configuration – ROEDY_LISTEN: …`). The names and meanings are the same as in
Roedy Mail.

| Variable | Default | Meaning |
|---|---|---|
| `ROEDY_PUBLIC_URL` | – | **Required.** Public address of the hub, e.g. `https://app.example.com`: scheme, host and optionally a port, without path, credentials, query or fragment. The hub owns the root of its origin; the apps live under `/<slug>/`. Without it, `serve` exits with `error: ROEDY_PUBLIC_URL is not set – …` before it creates anything. |
| `ROEDY_LISTEN` | `0.0.0.0:8080` | Address and port of the public HTTP server: web app, `/api/v1`, `/healthz`, `/readyz` and later the apps. The reverse proxy handles TLS. |
| `ROEDY_INTERNAL_LISTEN` | `0.0.0.0:8081` | Address and port of the internal HTTP server for the apps (`/internal/v1/…`). **Never publish this port** and never forward it through the reverse proxy. Set to empty to turn it off; then no app can pair. It must not overlap with `ROEDY_LISTEN`. Until pairing exists, it answers `404` to every request. |
| `ROEDY_DATA_DIR` | `data` relative to the working directory | Directory with the database `roedy.sqlite3` and the key `secret.key`; created with `0700`, files `0600`. Must be on a local file system (SQLite in WAL mode). |
| `ROEDY_LOG` | `info` | Log filter in the format of [`EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html), e.g. `warn` or `info,roedy_hub_server=debug`. `info,tower_http=debug` logs every request (method and path, tokens in paths replaced by `***`, never the query). The setup token always appears. |
| `ROEDY_LOG_FORMAT` | `json` | `json` (one line per event) or `text`. Logs go to stderr. |
| `ROEDY_WEB_DIR` | – | Directory with the web app build (e.g. `apps/web/build`). Takes precedence over the embedded web app; for development. |
| `ROEDY_TRUST_PROXY` | `false` | Reverse proxies in front of the server: `true` for one, otherwise their number (at most 10). The client IP is then the entry in `X-Forwarded-For` appended by the front-most proxy, and the server sends HSTS. Only set it when the public port is reachable exclusively via the proxy and every proxy appends to `X-Forwarded-For` instead of passing it through. |
| `ROEDY_SECRET_KEY` | – | Instance key for secrets in the database: 32 bytes as 64 hex characters (`openssl rand -hex 32`) or Base64 (`openssl rand -base64 32`). If it is missing as well as `ROEDY_SECRET_KEY_FILE`, the first start creates `<ROEDY_DATA_DIR>/secret.key` (`0600`) and uses it afterwards. Without the key the encrypted values can no longer be read; keep a copy outside the data directory. |
| `ROEDY_SECRET_KEY_FILE` | – | File with this key, e.g. a Docker secret (`/run/secrets/roedy_secret_key`). Not together with `ROEDY_SECRET_KEY`. |

**Initial setup:** as long as no administrator exists, every start writes a setup token to the log
(`Setup-Token: XXXX-XXXX-XXXX`). `roedy-hub setup-token` shows it again.

**Commands:** `roedy-hub serve` starts the server, `roedy-hub healthcheck` checks it ([Health](#health)),
`roedy-hub migrate` applies pending database migrations (`serve` does that as well) and `roedy-hub setup-token` shows
the setup token. All of them read the same variables. On `SIGTERM` or `SIGINT` the server stops accepting connections
on both ports and gives open requests 8 seconds to finish.

**Lost administrator access:** `roedy-hub reset-admin` restores an administrator's access – for example when the only
administrator lost both password and authenticator app. It sets a new random password and prints it, removes the
authenticator apps and recovery codes, signs out every device, discards open links from mails and unlocks the
account; the audit log records it. With several administrators, choose one with `--user <email address or username>`.
`--link` prints a link to set a new password (valid for an hour, needs `ROEDY_PUBLIC_URL`) instead of a password, so the
operator never learns it; the old password stops working right away. Run it next to the server with the same
`ROEDY_DATA_DIR`, for example in its container.

**Cleanup:** the server deletes expired sessions and links from mails every hour, and audit log entries older than the
retention chosen under Administration › Audit log (30, 90 or 365 days, default 90) once a day.

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
`::1`), or to the address itself if `ROEDY_LISTEN` names one. Use it as the container's health check:

```yaml
healthcheck:
  test: ["CMD", "roedy-hub", "healthcheck"]
  interval: 30s
  timeout: 5s
  start_period: 10s
  retries: 3
```
