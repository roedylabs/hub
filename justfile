# Tasks for development (https://just.systems). `just` lists all recipes.
# Without just installed: `pnpm exec just …` (package rust-just, pinned in
# package.json). Image, release and end-to-end recipes follow with the
# delivery milestone (docs/plan.md).

set shell := ["bash", "-euo", "pipefail", "-c"]

# List recipes
default:
    @just --list

# All checks as in the pre-push hook
check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    pnpm lint
    pnpm check
    pnpm test
