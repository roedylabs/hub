# Security policy

## Supported versions

Roedy Hub is in 0.x development. Security fixes are released for the latest published version only.

| Version | Supported |
|---|---|
| latest 0.x release | ✅ |
| older releases | ❌ |

## Reporting a vulnerability

Please do **not** report security vulnerabilities via public issues, pull requests or discussions.

Use GitHub's private vulnerability reporting: **[Report a vulnerability](https://github.com/roedylabs/hub/security/advisories/new)** (repository tab "Security" › "Report a vulnerability").

Please include the affected version or commit, a description and the impact, steps to reproduce and, if available, a suggested fix. We aim to acknowledge reports within a few days, keep you informed while we work on a fix, and agree on the time of disclosure with you. Reporters are credited in the advisory unless they prefer otherwise.

## Particularly relevant areas

- Gateway: forwarding to the apps, removal of client `X-Roedy-*` headers, cookie isolation between apps, WebSocket origin checks
- Identity: signing and verification of `X-Roedy-Identity`, key rotation, events to the apps
- Pairing of apps: pairing codes, app secrets, the internal listener
- Sign-in, sessions, two-factor sign-in, recovery codes, password reset, invitations, setup token
- Permissions per app, administration functions
- Content Security Policy and other security headers

## Notes for operators

- All apps behind the hub share one origin. A vulnerability in one app (such as cross-site scripting) affects every app and the hub. Only install apps you trust as much as the hub itself ([`docs/app-contract.md`](docs/app-contract.md#2-trust-model)).
- Never publish the internal listener (`ROEDY_INTERNAL_LISTEN`, port 8081) or the apps' own ports; only the hub's public port belongs behind the TLS proxy.
- Secrets do not belong in the repository or in images: use `ROEDY_SECRET_KEY` or `ROEDY_SECRET_KEY_FILE`, or back up the key `/data/secret.key` (mode 0600) created on first start.
- The reverse proxy terminates TLS; only set `ROEDY_TRUST_PROXY` when a trusted proxy sits in front.
- Anyone who can read the container logs can set up an instance that is not yet set up with the setup token. The token becomes invalid with the first administrator.
