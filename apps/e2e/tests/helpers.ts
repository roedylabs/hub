// Helpers of the end-to-end suite: browser contexts per view and theme,
// uncaught page errors, axe, TOTP codes for the mandatory second factor,
// passwords and the example app's pairing. axeViolations, totp and
// Authenticator are taken over from Roedy Mail (apps/web/e2e/helpers.ts).
import { expect, type Browser, type BrowserContext, type Page } from '@playwright/test';
import axe from 'axe-core';
import { execFile } from 'node:child_process';
import { createHmac, randomBytes } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

/** Signed-in session (cookies), in memory only. */
export type Session = Awaited<ReturnType<BrowserContext['storageState']>>;

/** Value from a previous test; if it is missing, the previous test failed. */
export function need<T>(value: T | undefined, what: string): T {
  if (value === undefined) throw new Error(`${what} missing from the previous test`);
  return value;
}

/** Viewports as in the Chromium audit (scripts/a11y/audit.mjs). */
export const VIEWS = {
  desktop: { width: 1440, height: 900 },
  mobile: { width: 390, height: 844 },
} as const;
export type View = keyof typeof VIEWS;
export type Theme = 'light' | 'dark';
/** Every combination of view and theme. */
export const LOOKS: readonly { view: View; theme: Theme }[] = (['desktop', 'mobile'] as const).flatMap((view) =>
  (['light', 'dark'] as const).map((theme) => ({ view, theme })),
);

/** Uncaught page errors; `takePageErrors()` in `afterEach` makes the test fail on them. */
const pageErrors: string[] = [];

/** A new context in this view and theme (the hub follows the system's colour scheme), optionally signed in. */
export async function newContext(
  browser: Browser,
  look: { view: View; theme: Theme },
  state?: Session,
): Promise<BrowserContext> {
  return browser.newContext({
    viewport: VIEWS[look.view],
    colorScheme: look.theme,
    reducedMotion: 'reduce',
    ...(state ? { storageState: state } : {}),
  });
}

/** A new page in `context` whose uncaught errors are collected. */
export async function newPage(context: BrowserContext): Promise<Page> {
  const page = await context.newPage();
  page.on('pageerror', (err) => pageErrors.push(`${page.url()}: ${err.message}`));
  return page;
}

/** The page errors collected since the last call (and clears the list). */
export function takePageErrors(): string[] {
  return pageErrors.splice(0);
}

/** Closes every context of `browser` (also those of a failed test) and returns the page errors. */
export async function closeAll(browser: Browser): Promise<string[]> {
  await Promise.all(browser.contexts().map((c) => c.close()));
  return takePageErrors();
}

/** WCAG 2.2 AA levels and `best-practice`, the rule set of the Chromium audit (`scripts/a11y/audit.mjs`). */
const AXE_TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'];

/**
 * axe-core findings on the page, per node with rule, target and HTML; empty
 * if everything passes. Only after running animations have ended (a fade-in
 * would otherwise count as too little contrast); endless ones like spinners do
 * not count. axe gets into the page via `evaluate`, not as `<script>` (the
 * hub's CSP would not allow it).
 */
export async function axeViolations(page: Page): Promise<string[]> {
  try {
    await page.waitForFunction(
      () =>
        document
          .getAnimations()
          .every((a) => a.playState !== 'running' || a.effect?.getTiming().iterations === Infinity),
      null,
      { timeout: 5000, polling: 100 },
    );
  } catch {
    const running = await page.evaluate(() =>
      document
        .getAnimations()
        .filter((a) => a.playState === 'running')
        .map((a) =>
          a instanceof CSSAnimation ? a.animationName : a instanceof CSSTransition ? a.transitionProperty : a.id,
        ),
    );
    throw new Error(`axe: animations still running after 5 s: ${running.join(', ')}`);
  }
  if (!(await page.evaluate(() => 'axe' in window))) await page.evaluate(axe.source);
  return page.evaluate(async (tags) => {
    const run = (window as unknown as { axe: typeof axe }).axe.run;
    const r = await run(document, { runOnly: { type: 'tag', values: tags }, resultTypes: ['violations'] });
    return r.violations.flatMap((v) =>
      v.nodes.map((n) => `${v.id} ${n.target.join(' ')}: ${n.html.slice(0, 160)} – ${n.failureSummary ?? ''}`),
    );
  }, AXE_TAGS);
}

/** Asserts that axe finds nothing on the page; `what` names the page state in the failure. */
export async function expectAccessible(page: Page, what: string): Promise<void> {
  expect(await axeViolations(page), `axe: ${what}`).toEqual([]);
}

/** Base32 (RFC 4648) without padding; spaces and lowercase letters allowed. */
function base32(secret: string): Buffer {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  let bits = '';
  for (const char of secret.replace(/[\s=]/g, '').toUpperCase()) {
    const value = alphabet.indexOf(char);
    if (value < 0) throw new Error('Key is not Base32');
    bits += value.toString(2).padStart(5, '0');
  }
  const bytes = bits.match(/.{8}/g) ?? [];
  return Buffer.from(bytes.map((b) => parseInt(b, 2)));
}

/** TOTP per RFC 6238 (SHA-1, 6 digits, 30 s) like the authenticator apps. */
export function totp(secret: string, now = Date.now()): string {
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(now / 30_000)));
  const hmac = createHmac('sha1', base32(secret)).update(counter).digest();
  const offset = hmac.readUInt8(hmac.length - 1) & 0x0f;
  const code = (hmac.readUInt32BE(offset) & 0x7fffffff) % 1_000_000;
  return code.toString().padStart(6, '0');
}

/** Time step of a TOTP code in ms. */
const TOTP_STEP = 30_000;

/**
 * Authenticator app for `secret` whose codes the server accepts: each code is valid once, its time step must be newer
 * than the last accepted one (`used`, e.g. the time of the setup's confirmation).
 */
export class Authenticator {
  readonly #secret: string;
  #last: number;

  constructor(secret: string, used = Date.now()) {
    this.#secret = secret;
    this.#last = Math.floor(used / TOTP_STEP);
  }

  /**
   * Next code. The server accepts one step before and after the current one, so this takes the step before while
   * enough of the current one is left for the request, and waits for a code two steps ahead until it is one ahead.
   */
  async next(): Promise<string> {
    const t = Date.now();
    const now = Math.floor(t / TOTP_STEP);
    const at = Math.max(t % TOTP_STEP < TOTP_STEP - 5_000 ? now - 1 : now, this.#last + 1);
    // Valid from step `at - 1` on, with a second's margin for the server's clock.
    const wait = (at - 1) * TOTP_STEP + 1_000 - Date.now();
    if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait));
    this.#last = at;
    return totp(this.#secret, at * TOTP_STEP);
  }
}

/** Random password following the policy (≥ 10 characters, upper, lower, non-letter). */
export function password(): string {
  return `Rh-${randomBytes(12).toString('base64url')}-aZ7`;
}

/** scripts/e2e.sh of this checkout. */
const E2E_SCRIPT = fileURLToPath(new URL('../../../scripts/e2e.sh', import.meta.url));

/**
 * (Re)starts the example app paired with `code`: `scripts/e2e.sh --pair`
 * recreates its container with the code, or restarts the binary on the host,
 * as the running scripts/e2e.sh started it. No shell: the code is passed in
 * the environment only.
 */
export async function pairExample(code: string): Promise<void> {
  await promisify(execFile)(E2E_SCRIPT, ['--pair'], {
    env: { ...process.env, ROEDY_EXAMPLE_PAIRING_CODE: code },
    timeout: 90_000,
  });
}
