# Contributing to Roedy Hub

Thanks for your interest in Roedy Hub! Bug reports, ideas, translations, documentation and code are all welcome.

- **Questions and ideas:** [GitHub Discussions](https://github.com/roedylabs/hub/discussions)
- **Bugs and feature requests:** [issues](https://github.com/roedylabs/hub/issues/new/choose)
- **Security vulnerabilities:** never in public, see [SECURITY.md](SECURITY.md)

For larger changes, please open an issue or discussion first so we can agree on the approach before you invest time.
Changes to the app contract ([`docs/app-contract.md`](docs/app-contract.md)) affect every Roedy app; discuss them first.

## Development setup

**Requirements:** Node 26, pnpm 12 and Rust stable. The easiest way is [mise](https://mise.jdx.dev) (`mise install`). Without mise, enable pnpm via corepack (`npm install -g corepack && corepack enable`); the pnpm version is pinned in `package.json`.

```sh
pnpm install          # dependencies and Git hooks (lefthook)
ROEDY_PUBLIC_URL=http://localhost:5173 ROEDY_LOG_FORMAT=text cargo run -- serve   # server on :8080 and :8081, data in ./data
pnpm dev              # web app on http://localhost:5173, proxies /api and the health checks to the server
```

`ROEDY_PUBLIC_URL` is required; in development it is the address of the web app. The first start prints a setup token in the server log (`cargo run -- setup-token` shows it again). The other commands are `healthcheck`, `migrate` and `reset-admin`; all settings are in [`docs/ops.md`](docs/ops.md#configuration). To try the server with the built web app instead of `pnpm dev`, run `pnpm build` and start it with `ROEDY_WEB_DIR=apps/web/build` (or build it with `--features embed-web`, as the image will).

Useful environment variables during development: `ROEDY_LOG_FORMAT=text` for readable logs, `ROEDY_LOG=debug,tower_http=debug` to log every request, `ROEDY_INTERNAL_LISTEN=` (empty) to turn the internal listener off.

In a sandboxed shell with a read-only home directory, run `. scripts/dev/sandbox-env.sh` first; it moves the pnpm, npm and cargo caches to `$TMPDIR`.

Common tasks are in the [`justfile`](justfile) (`pnpm exec just …` if [just](https://just.systems) is not installed):

| Command | What it does |
|---|---|
| `just check` | All checks below |
| `just dist [<targets>]` | Web app and static musl binaries to `dist/` (needs Zig, cargo-zigbuild, cargo-auditable and the rustup targets) |
| `just image` / `just smoke` | Image for the local platform from `dist/` and its smoke test with the budgets |
| `just dist-example`, `just image-example`, `just smoke-compose` | The example app's image and the smoke test of `deploy/compose.yml` (the smoke tests need curl, openssl and, for Compose, jq) |
| `just e2e` / `just e2e-host` | The [end-to-end suite](#end-to-end-suite) against the Compose test setup or against debug binaries on the host |
| `just release <version>` / `just release-tag <version>` | Release PR, then the tag on `main` ([`release.yml`](.github/workflows/release.yml)) |

## Checks

The same checks run in CI and in the `pre-push` hook:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm lint             # ESLint and Prettier (pnpm format fixes formatting)
pnpm check            # svelte-check and TypeScript
pnpm test             # Vitest and script tests
```

CI also runs Clippy and the tests with `--all-features`, which embeds `apps/web/build` (`embed-web`); run `pnpm build` first to do the same locally.

While iterating, run single test files: `pnpm --filter @roedy/hub-web exec vitest run <file>` or `cargo test -p roedy-hub-server <filter>`.

### End-to-end suite

[`apps/e2e`](apps/e2e) is a Playwright suite with axe against the hub and the example app as
[`deploy/compose.test.yml`](deploy/compose.test.yml) runs them: HTTPS with a self-signed certificate for `localhost`
on `127.0.0.1:8443`, plain HTTP on `127.0.0.1:8080`. One serial flow builds on itself: the redirect to HTTPS and
HSTS, the setup with the authenticator app (codes from the shown key), sign-in with password and code, pairing the
example app from Administration › Apps, its tile on the home page opening it through the gateway with the identity,
SSE and WebSocket in the browser, a user without access getting the 403 page, every page state on desktop and mobile
in light and dark, and sign-out closing the app's streams with `Clear-Site-Data`. axe checks every page state with
the rule set of the Chromium audit (`scripts/a11y/`). CI runs it in the job "End-to-end" against the images the job
"Image" built.

[`scripts/e2e.sh`](scripts/e2e.sh) starts everything, passes it to the suite and stops it again; never run
`playwright test` directly. Once, install the browser:
`pnpm --filter @roedy/hub-e2e exec playwright install chromium-headless-shell` (or `scripts/dev/browser.sh install`,
whose browser the script finds as well). Ports 8443 and 8080 on `127.0.0.1` must be free.

```sh
# Against the containers, as in CI (needs Docker with Compose, or Podman: ROEDY_E2E_COMPOSE="podman compose")
just dist x86_64-unknown-linux-musl && just dist-example x86_64-unknown-linux-musl
just image && just image-example
pnpm e2e                         # = scripts/e2e.sh

# Without containers: debug binaries in target/e2e (also uses 8081 and 8090)
pnpm e2e:host                    # builds the web app, the hub and the example app first
scripts/e2e.sh --host --no-build -g 'setup|sign-in|pairing|sign-out'   # reuse the binaries; arguments go to `playwright test`
```

The browser trusts the test certificate only (`--ignore-certificate-errors-spki-list` with its key's hash) and Node
gets it as `NODE_EXTRA_CA_CERTS`; TLS checks stay on. The example app pairs through `ROEDY_E2E_PAIR`, a command
the script sets (`docker compose up` with the code, or a restart of the binary). On a failure,
`apps/e2e/test-results/` holds a trace, a screenshot and the page's accessibility snapshot per failed test plus the
hub's and the example's log (`compose.log` or `host.log`); CI uploads it as the artifact `e2e-results`. Open a trace
with `pnpm --filter @roedy/hub-e2e exec playwright show-trace <trace.zip>`; add `--headed` or `--debug` to watch a
run locally. The tests build on each other against one fresh hub: with `-g`, also select the ones a test needs (as in
the example above for the sign-out).

## Releases

Releases need the [GitHub CLI](https://cli.github.com), signed in with push access (`gh auth status`); `just release`
checks that before it changes anything. `just release <version>` opens the release PR (version, `CHANGELOG.md` with
git-cliff); after the maintainer merged it,
`just release-tag <version>` sets the tag on `main`, and [`release.yml`](.github/workflows/release.yml) builds, checks,
signs and publishes `ghcr.io/roedylabs/hub`. Only a pushed tag publishes; a manual run of the workflow is a dry run.
`x.y` and `latest` move only to the newest stable version, whatever order releases publish in.

**After the first release:** GitHub creates the package `hub` as private, and the workflow cannot change that. A
maintainer makes it public once (the package's settings › Danger Zone › Change visibility), so that `docker compose
pull` works without login as `docs/ops.md` describes; until then the workflow's step "Image pullable without login"
warns. Check it with `gh api orgs/roedylabs/packages/container/hub --jq .visibility`.

## Project layout

| Path | Content |
|---|---|
| `crates/server` | Server (Rust, axum, SQLite), binary `roedy-hub` |
| `crates/contract` | The app contract's types, signing and verification (`roedy-hub-contract`); makes and checks the test vectors in `docs/app-contract/vectors.json` |
| `crates/example-app` | The example app (`roedy-hub-example`): a test app that implements the contract, for the hub's tests; not part of the image |
| `apps/web` | Web app of the hub (Svelte 5, SvelteKit as an SPA) |
| `apps/e2e` | End-to-end suite (Playwright with axe) against the hub and the example app |
| `packages/tokens` | Design tokens as CSS and TypeScript constants, a copy from Roedy Mail ([origin](packages/tokens/README.md)) |
| `docs/` | App contract, plan, operator documentation ([`ops.md`](docs/ops.md)) |
| `deploy/` | Docker Compose setup (`compose.yml`, `.env.example`; the hub with built-in TLS, no proxy), `restore.sh` for backups, the test override with the example app and TLS from a test certificate |
| `Dockerfile` | The image: packages `dist/<arch>/roedy-hub` on `scratch` |
| `scripts/` | Git hooks, smoke tests of the image and the Compose setup, release script and developer tooling |

## Conventions

- **Language:** code, comments, commits and docs are in English. UI texts live in `apps/web/messages/{en,de}.json`. English is the source language; every key needs both languages.
- **Web app:** imports via `#lib/…` with file extension, colours through the design tokens.
- **Accessibility:** every interactive element must be usable with the keyboard and a screen reader, in the light and the dark theme.
- **Database:** never edit an existing migration; add a new one.
- **Dependencies:** keep the hub lightweight (image ≤ 25 MB, idle memory ≤ 40 MB). Justify new dependencies in the PR. Libraries that end up in the browser bundle belong in `dependencies`, build and test tools in `devDependencies`.
- **Pinning:** GitHub Actions by full commit SHA, Docker images in Dockerfiles by tag and digest; Dependabot keeps them current. `scripts/action-pins.test.mjs` and `scripts/docker-pins.test.mjs` check it in `pnpm test`.
- **Origin of code:** code from [Roedy Mail](https://github.com/roedylabs/mail) (same project, same licence) may be taken over and adapted; say so in the module comment. Do not copy code from other projects.

## Commits and pull requests

- Branch from `main` with a short-lived branch, e.g. `feat/pairing` or `fix/cookie-filter`.
- Commit messages and PR titles follow [Conventional Commits](https://www.conventionalcommits.org/en/): `<type>(<scope>): <description>` with the types `feat fix perf refactor docs test build ci chore style revert` and the scopes `server gateway contract example web tokens ci deploy docs deps repo`. The description starts lowercase and has no trailing period.
- Pull requests are squash-merged; the PR title becomes the commit and the changelog entry.
- Keep pull requests focused. Add tests, update the docs and include screenshots for UI changes (light and dark, desktop and mobile).
- CI, CodeQL and an automated review by CodeRabbit run on every pull request; a maintainer reviews and merges.

## License

Roedy Hub is licensed under the [GNU Affero General Public License v3.0 only](LICENSE). By submitting a contribution you agree that it is licensed under the same terms.
