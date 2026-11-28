# roedy-hub-contract

The Roedy Hub app contract, version 1, as shared Rust types: what an app
needs to sit behind the hub's gateway without reading the hub's code. The
contract itself is
[`docs/app-contract.md`](https://github.com/roedylabs/hub/blob/main/docs/app-contract.md)
in the hub's repository.

- **Identity and events:** `Identity` (the signed header the hub sends with
  every request, § 4) and `Event` (§ 10.1), verified with `verify` against the
  hub's public keys (`Jwks`, § 4.4).
- **Strict JWS** on top of `ring` (Ed25519), not a JWT library: `alg` must be
  `EdDSA` whatever the key says, `typ` decides which claims a token can carry
  (an event never passes as an identity and the other way round), and the
  times are checked with the contract's leeway. `SigningKey::sign` is the
  hub's side.
- **Manifest** (`Manifest`, § 8) with its rules, and the bodies of the
  internal API and pairing (`pairing`, § 7 and § 9).

The hub and its example app use this crate, so all three agree. The test
vectors in `docs/app-contract/vectors.json` (§ 15) are made and checked with
it; apps in other languages can check their verification against them.

## Versions

The crate is published with every hub release under the hub's version.
Before the first release (`0.x`) the contract may still change; afterwards it
stays backwards compatible within version 1.

## Licence

AGPL-3.0-only, like Roedy Hub.
