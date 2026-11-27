// What scripts/e2e.sh passes to the suite: where the hub runs, the test
// certificate it serves, the setup token and how to pair the example app.
// The suite never starts anything itself, so that the same tests run against
// the Compose setup and against the binaries on the host.
import { readFileSync } from 'node:fs';

function need(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is not set – start the suite with scripts/e2e.sh`);
  return value;
}

export const env = {
  /** The hub's public URL, `https://localhost:8443`. */
  get url(): string {
    return need('ROEDY_E2E_URL');
  },
  /** The hub's plain HTTP port (redirect and ACME only), `http://127.0.0.1:8080`. */
  get httpUrl(): string {
    return need('ROEDY_E2E_HTTP_URL');
  },
  /** The self-signed certificate of scripts/test-cert.sh (PEM). */
  get cert(): string {
    return readFileSync(need('ROEDY_E2E_CERT'), 'utf8');
  },
  /** The setup token of the fresh hub. */
  get setupToken(): string {
    return need('ROEDY_E2E_SETUP_TOKEN');
  },
  /** Shell command that (re)starts the example app paired with `ROEDY_EXAMPLE_PAIRING_CODE`. */
  get pair(): string {
    return need('ROEDY_E2E_PAIR');
  },
  /** Where the hub reaches the example app, entered in Administration › Apps. */
  get exampleInternal(): string {
    return need('ROEDY_E2E_EXAMPLE_INTERNAL');
  },
};
