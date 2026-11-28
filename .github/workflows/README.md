# Workflows

| Workflow | Runs on | What it does |
| --- | --- | --- |
| [CI](ci.yml) | PR, merge queue, main, weekly | Lint (ESLint, Prettier, ShellCheck, actionlint), web app (types, tests, build), server (rustfmt, Clippy, tests), dependencies (cargo deny, licenses), packages (the npm tarballs in an app outside the workspace, `npm publish --dry-run`, `cargo package`), image (amd64 build, size ≤ 25 MB, smoke test with idle memory ≤ 40 MB, Compose setup), end-to-end (Playwright with axe against that image on the Compose test setup) |
| [Release](release.yml) | tag `v*`, manually | Both platforms, size, smoke test, Trivy; on tags: push to ghcr.io with SBOM and provenance, cosign, attestations, GitHub release, then the npm packages (with provenance) and the crates |
| [Pull request](pr.yml) | PR | Conventional Commit title, dependency review |
| [CodeQL](codeql.yml) | PR, main, weekly | Code scanning of workflows, TypeScript and Rust |
| [Scorecard](scorecard.yml) | main, weekly | OpenSSF Scorecard |

## Required checks

The ruleset of `main` requires these job names; renaming a job breaks merging
until the ruleset is updated as well:

- `CI passed` (aggregates all CI jobs)
- `Conventional title`
- `Analyze (actions)`, `Analyze (javascript-typescript)`, `Analyze (rust)`

## Conventions

- Every `uses:` is pinned to a full commit SHA with the version as a comment
  (checked by `scripts/action-pins.test.mjs`); Dependabot updates both.
- `permissions` are read-only at the top and widened per job only where needed.
- Every job has a `timeout-minutes`, every step a `name`.
- Checkouts use `persist-credentials: false`.
- Shared setup lives in composite actions in [`../actions/`](../actions):
  `setup-node` (pnpm, Node.js, `pnpm install`), `setup-rust` (stable
  toolchain, build cache written only on main), `setup-dist` (musl targets,
  Zig, cargo-zigbuild, cargo-auditable for `just dist`) and `setup-buildx`
  (Buildx with the BuildKit image pinned in `../buildkit/Dockerfile`).
- Pull request runs are cancelled by newer pushes; runs on main and tags are not.
