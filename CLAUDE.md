# Roedy Hub

Gateway, central sign-in, home page and shared platform for the Roedy apps (Mail, Calendar, Contacts …) under one
origin: everything several apps need is built here, the apps (separate containers and repositories) keep only what is
specific to them (`docs/plan.md` § 9). Backend: Rust (axum, SQLite) in `crates/server`, binary `roedy-hub`. Web app:
Svelte 5 / SvelteKit SPA in `apps/web`. Packages the hub publishes for the apps (`docs/plan.md` § 9.4), used by the web
app from the workspace: `packages/tokens` (`@roedy/tokens`, design tokens, the source), `packages/ui` (`@roedy/ui`,
shared Svelte components, actions and helpers with their own en/de texts), `packages/jmap` (`@roedy/jmap`, JMAP for the
browser). Toolchain: Node 26, pnpm 12, Rust stable.
`../mail` is the archived standalone Roedy Mail (`mail-standalone`): a reference only, read-only from here.

## Key documents

- `docs/app-contract.md`: the contract apps implement without reading the hub's code. Keep it and the code in sync;
  until the first release it may change freely (still version 1), afterwards changes stay backwards compatible within
  version 1.
- `docs/plan.md`: architecture, milestones (one PR each) and open points.

## Commands

- Sandboxed shell with read-only home: `. scripts/dev/sandbox-env.sh` in the same shell call before pnpm/cargo.
- Run: `pnpm install`, `ROEDY_PUBLIC_URL=http://localhost:5173 cargo run -- serve` (:8080, internal :8081), `pnpm dev` (:5173).
- Checks (all must pass before a PR): `pnpm lint`, `pnpm check`, `pnpm test`,
  `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Single tests: `pnpm --filter @roedy/hub-web exec vitest run <file>` (or `@roedy/ui`, `@roedy/jmap`),
  `cargo test -p roedy-hub-server <filter>`.
- End-to-end (`apps/e2e`, CI job "End-to-end"): `scripts/e2e.sh` needs Docker; without it `scripts/e2e.sh --host`.
- `just check` (`pnpm exec just …` without just).

## Conventions (details in CONTRIBUTING.md)

- English everywhere. German only in `apps/web/messages/de.json`, the `texts()` tables of `packages/ui` (and later the
  server's locales and mail templates); every message key exists in en and de.
- Shared by several apps → `packages/ui` (or `jmap`/`tokens`), not `apps/web`; the web app imports it from `@roedy/ui`.
- Web: `#lib/…` imports with file extension, colours via design tokens; keyboard and screen-reader accessible,
  light and dark theme.
- Database: until the first release there is one migration, the initial schema `0001_init.sql`; edit it in place. After
  the first release, never edit a released migration; add a new one.
- Stay lightweight (image ≤ 25 MB, idle ≤ 40 MB); justify new dependencies.
- Code may be taken from Roedy Mail (same project and licence) with a note of origin; never from other projects.
- Design: sign-in, setup and invitation follow the new "Roedy Mail" design, account and administration the current
  Roedy Mail app; ask before finalising the home page design.

## Git

- Never push to `main`; work on a short-lived branch and open a PR. The maintainer squash-merges; never merge yourself.
- Never create, push to or publish a GitHub repository, issue or release without asking. Issues only as drafts.
- Conventional Commits for commits and PR titles: `<type>(<scope>): <description>`, lowercase, no trailing period.
  Types: feat fix perf refactor docs test build ci chore style revert.
  Scopes: server gateway contract example web tokens ui jmap ci deploy docs deps repo release.
  The PR title becomes the commit and the changelog entry.
- No real names, domains or email addresses in commits, docs, tests or fixtures. Use `example.com`, `*.example`,
  `*.test`, `*.invalid`; commit identities use GitHub noreply addresses.
