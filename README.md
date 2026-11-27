# Roedy Hub

**One address for all Roedy apps: gateway, central sign-in and home page.**

> **Status:** early development, nothing to install yet. The plan is in
> [`docs/plan.md`](docs/plan.md).

```text
https://app.example.com/            → Roedy Hub: home page with all installed apps
https://app.example.com/mail/…      → Roedy Mail
https://app.example.com/contacts/…  → Roedy Contacts (later)
```

Roedy Hub sits in front of the Roedy apps and is three things at once:

- **Gateway:** forwards `/<app>/…` to the app's container, with streaming, Server-Sent Events and WebSockets.
- **Central sign-in:** users, passwords, two-factor sign-in, sessions, invitations and permissions per app. Apps receive a short-lived signed identity with every request and keep no passwords of their own.
- **Home page:** the apps a person may use, their account and the administration.

Apps keep working on their own without the hub. How an app connects to the hub is described in the
[app contract](docs/app-contract.md).

## How it works

```text
Browser ──HTTPS──► roedy-hub ─┬─ /, /login, /account, /admin, /api/v1   (the hub itself)
                              ├─ /mail/…      ──► roedy-mail      (+ signed identity)
                              └─ /contacts/…  ──► roedy-contacts
```

All apps share one origin. The hub terminates TLS itself and gets its certificate from Let's Encrypt (or another ACME
CA) on its own ([TLS](docs/ops.md#tls)). Install only apps you trust as much as the hub itself
([trust model](docs/app-contract.md#2-trust-model)).

## Development

```sh
pnpm install
ROEDY_PUBLIC_URL=http://localhost:5173 cargo run -- serve
pnpm dev
```

Requires Node 26, pnpm 12 and Rust stable ([mise](https://mise.jdx.dev) installs them with `mise install`). See
[CONTRIBUTING.md](CONTRIBUTING.md) for the project layout, checks and conventions, and [`docs/ops.md`](docs/ops.md) for
the configuration.

| Part | Stack |
|---|---|
| Server | Rust, axum, tokio, SQLite (rusqlite) |
| Web app | Svelte 5 (runes), SvelteKit as a SPA, TypeScript, Vite, Paraglide i18n |
| Delivery | Static musl binary on `scratch`, multi-arch image |

## License

Roedy Hub is free software under the [GNU Affero General Public License v3.0 only](LICENSE) (`AGPL-3.0-only`).
