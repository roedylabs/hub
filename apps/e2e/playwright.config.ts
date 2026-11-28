// End-to-end suite against the hub and the example app as deployed by
// deploy/compose.test.yml (HTTPS with a self-signed certificate). Start it only
// through `scripts/e2e.sh` (locally and in CI): the script starts the hub and
// the example app and passes them in the environment (env.ts).
import { defineConfig } from '@playwright/test';
import { X509Certificate, createHash } from 'node:crypto';
import { env } from './env.ts';

const ci = !!process.env.CI;

/**
 * The browser trusts the test certificate and nothing else besides the usual
 * roots: Chromium accepts a certificate whose public key has this SHA-256
 * (`--ignore-certificate-errors-spki-list`, honoured with the profile
 * directory Playwright always passes). TLS checks stay on for everything else –
 * no `ignoreHTTPSErrors`.
 */
function spki(pem: string): string {
  const key = new X509Certificate(pem).publicKey.export({ type: 'spki', format: 'der' });
  return createHash('sha256').update(key).digest('base64');
}

export default defineConfig({
  testDir: 'tests',
  // One hub for the whole run, and flows that build on each other.
  workers: 1,
  fullyParallel: false,
  forbidOnly: ci,
  retries: 0,
  timeout: 90_000,
  expect: { timeout: 15_000 },
  reporter: ci ? [['list'], ['github']] : 'list',
  use: {
    baseURL: env.url,
    locale: 'en-US',
    timezoneId: 'Europe/Berlin',
    // A click on a missing element fails after 15 s instead of only with the test.
    actionTimeout: 15_000,
    navigationTimeout: 30_000,
    launchOptions: { args: [`--ignore-certificate-errors-spki-list=${spki(env.cert)}`] },
    // Traces and screenshots of failures: CI uploads test-results/ as an
    // artifact. They show the run's passwords and TOTP secrets, which belong
    // to a throwaway hub that is removed after the run.
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
});
