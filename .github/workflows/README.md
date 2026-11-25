# Workflows

| Workflow | Runs on | What it does |
| --- | --- | --- |
| [CI](ci.yml) | PR, merge queue, main, weekly | Lint (ESLint, Prettier, ShellCheck, actionlint), web app (types, tests, build), server (rustfmt, Clippy, tests), dependencies (cargo deny, licenses) |
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
  `setup-node` (pnpm, Node.js, `pnpm install`) and `setup-rust` (stable
  toolchain, build cache written only on main).
- Pull request runs are cancelled by newer pushes; runs on main and tags are not.
