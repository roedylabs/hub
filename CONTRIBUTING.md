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
cargo run -- --version
pnpm dev              # web app on http://localhost:5173, proxies /api to the backend on :8080
```

The server commands (`serve`, `healthcheck`, …) follow in the next milestones ([`docs/plan.md`](docs/plan.md)).

In a sandboxed shell with a read-only home directory, run `. scripts/dev/sandbox-env.sh` first; it moves the pnpm, npm and cargo caches to `$TMPDIR`.

Common tasks are in the [`justfile`](justfile) (`pnpm exec just …` if [just](https://just.systems) is not installed):

| Command | What it does |
|---|---|
| `just check` | All checks below |

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

While iterating, run single test files: `pnpm --filter @roedy/hub-web exec vitest run <file>` or `cargo test -p roedy-hub-server <filter>`.

## Project layout

| Path | Content |
|---|---|
| `crates/server` | Server (Rust, axum, SQLite), binary `roedy-hub` |
| `apps/web` | Web app of the hub (Svelte 5, SvelteKit as an SPA) |
| `packages/tokens` | Design tokens as CSS and TypeScript constants, a copy from Roedy Mail ([origin](packages/tokens/README.md)) |
| `docs/` | App contract, plan, later the operator documentation |
| `scripts/` | Git hooks and developer tooling |

`crates/contract` (the contract's types, signing and verification) and `crates/example-app` (a test app that implements the contract) follow with the gateway.

## Conventions

- **Language:** code, comments, commits and docs are in English. UI texts live in `apps/web/messages/{en,de}.json`. English is the source language; every key needs both languages.
- **Web app:** imports via `#lib/…` with file extension, colours through the design tokens.
- **Accessibility:** every interactive element must be usable with the keyboard and a screen reader, in the light and the dark theme.
- **Database:** never edit an existing migration; add a new one.
- **Dependencies:** keep the hub lightweight (image ≤ 25 MB, idle memory ≤ 40 MB). Justify new dependencies in the PR. Libraries that end up in the browser bundle belong in `dependencies`, build and test tools in `devDependencies`.
- **Pinning:** GitHub Actions by full commit SHA; Dependabot keeps them current. `scripts/action-pins.test.mjs` checks it in `pnpm test`.
- **Origin of code:** code from [Roedy Mail](https://github.com/roedylabs/mail) (same project, same licence) may be taken over and adapted; say so in the module comment. Do not copy code from other projects.

## Commits and pull requests

- Branch from `main` with a short-lived branch, e.g. `feat/pairing` or `fix/cookie-filter`.
- Commit messages and PR titles follow [Conventional Commits](https://www.conventionalcommits.org/en/): `<type>(<scope>): <description>` with the types `feat fix perf refactor docs test build ci chore style revert` and the scopes `server gateway contract example web tokens ci deploy docs deps repo`. The description starts lowercase and has no trailing period.
- Pull requests are squash-merged; the PR title becomes the commit and the changelog entry.
- Keep pull requests focused. Add tests, update the docs and include screenshots for UI changes (light and dark, desktop and mobile).
- CI, CodeQL and an automated review by CodeRabbit run on every pull request; a maintainer reviews and merges.

## License

Roedy Hub is licensed under the [GNU Affero General Public License v3.0 only](LICENSE). By submitting a contribution you agree that it is licensed under the same terms.
