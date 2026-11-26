// Accessibility audit and screenshots of the hub's pages: axe-core in a headless
// Chromium against the Vite dev server, every page state on desktop 1440 × 900
// and mobile 390 × 844, each in the light and the dark theme. The API is
// answered with made-up data (`mock` below); nothing reaches a server.
//
// Usage (starts the Vite dev server for the run):
//   scripts/dev/browser.sh run scripts/a11y/audit.mjs <output-dir> [--only <part-of-id>[,…]] [--no-shots]
// Prerequisite: scripts/dev/browser.sh install (Playwright and axe-core).
//
// Result: <output-dir>/report.json (all findings), report.md (summary) and,
// unless --no-shots, one screenshot per state in <output-dir>/screenshots/.
// The exit code is 1 if any state has a finding, a page error or could not be audited.
//
// Taken over from Roedy Mail (scripts/a11y/audit.mjs) and trimmed to the axe
// run over the hub's pages before and around the sign-in.
import axe from 'axe-core';
import fs from 'node:fs';
import path from 'node:path';
import { chromium } from 'playwright';

const BASE = process.env.BASE ?? 'http://127.0.0.1:5173';
const AXE_TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'];

const argv = process.argv.slice(2);
const flag = (name) => {
  const i = argv.indexOf(name);
  return i >= 0 ? (argv.splice(i, 2)[1] ?? '') : null;
};
const only = (flag('--only') ?? '').split(',').filter(Boolean);
const shots = !argv.includes('--no-shots');
const outDir = argv.find((a) => !a.startsWith('--'));
if (!outDir) {
  console.error('Usage: audit.mjs <output-dir> [--only …] [--no-shots]');
  process.exit(2);
}
const OUT = path.resolve(outDir);
const PHOTOS = path.join(OUT, 'screenshots');
fs.mkdirSync(PHOTOS, { recursive: true });

const VIEWS = {
  desktop: { width: 1440, height: 900 },
  mobile: { width: 390, height: 844 },
};
const THEMES = ['light', 'dark'];

// --- Made-up data (only against the mocked API) ---------------------------

const BRANDING = {
  kind: 'standard',
  letter: '',
  icon: 'ti-mountain',
  image: null,
  fit: 'padded',
  plate: 'accent',
  tilt: true,
  color: 'apricot',
  userAccent: true,
};
const ORG = {
  name: 'Example Org',
  website: null,
  help: null,
  email: 'support@example.com',
  imprint: 'https://example.com/imprint',
  privacy: 'https://example.com/privacy',
  terms: null,
  source: null,
};
const CONFIG = { name: 'Example Org', branding: BRANDING, org: ORG, setup_required: false };
const USER = {
  id: '01JUSER',
  username: 'alex',
  email: 'alex@example.com',
  name: 'Alex Example',
  role: 'admin',
  locale: 'en',
  theme: 'system',
};
const SESSION = {
  user: USER,
  session: { id: '01JSESSION', auth_time: 1795600000 },
  hub: { name: 'Example Org', home: '/', account: '/account', admin: '/admin' },
};
const SECRET = 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP';
const ENROLL = { secret: SECRET, uri: `otpauth://totp/Example%20Org:alex%40example.com?secret=${SECRET}` };
const CODES = Array.from({ length: 10 }, (_, i) => `K7PX-${String(4000 + i * 137).slice(0, 4)}`);
const INVITE = {
  email: 'jordan@example.com',
  name: 'Jordan Hale',
  role: 'user',
  inviter: 'Alex Example',
  expires_at: '2026-12-03T10:00:00Z',
  expired: false,
};
const PASSWORD = 'Example-Password-2026!';

/**
 * Answers of the API per scenario: `METHOD path` → `{ status, body }`, a
 * function of the request, or a list of them (one per request, the last one
 * repeats). Everything else gets 404 `not_found`.
 */
function mock({ setup = false, signedIn = false, routes = {} } = {}) {
  return {
    'GET public/config': { body: { ...CONFIG, setup_required: setup } },
    'GET session': signedIn ? { body: SESSION } : { status: 401, body: { error: 'unauthorized' } },
    'GET settings': { body: { version: 1, settings: { lang: 'en', mode: null, accent: null } } },
    'POST auth/logout': { status: 204 },
    ...routes,
  };
}

// --- Scenarios --------------------------------------------------------------

const setupApi = {
  'POST setup/token': { body: {} },
  'POST setup/totp': { body: ENROLL },
  'POST setup/totp/verify': { body: { recovery_codes: CODES } },
  'POST setup/complete': { status: 201, body: { user: {} } },
};

const fill = (page, selector, value) => page.locator(selector).fill(value);
/**
 * The screen's primary button: `type=submit` in the card on desktop, `button.primary` in the bottom bar on mobile.
 * A dispatched click: Playwright's hit test does not always reach the bottom bar while it settles.
 */
const submit = (page) => page.locator('button.primary').filter({ visible: true }).last().dispatchEvent('click');
const button = (page, name) => page.getByRole('button', { name, exact: true }).first().dispatchEvent('click');

/** Fills the visible fields of the open setup step, ticks its checkbox and continues. */
async function setupStep(page, n) {
  const values = {
    'su-token': 'ABCD-EFGH-JKLM',
    'su-name': 'Alex Example',
    'su-email': 'alex@example.com',
    'su-pw': PASSWORD,
    'su-pw2': PASSWORD,
    'su-otp': '123456',
  };
  for (let guard = 0; guard < 12; guard++) {
    const label =
      (await page
        .locator('.bars')
        .first()
        .getAttribute('aria-label')
        .catch(() => null)) ?? '';
    const at = Number(/Step (\d)/.exec(label)?.[1] ?? 0);
    if (at === n) return;
    if (at > n || !at) throw new Error(`setup: step ${n} not reached (at ${at})`);
    for (const [id, value] of Object.entries(values)) {
      const field = page.locator(`#${id}`);
      if ((await field.count()) && !(await field.inputValue())) await field.fill(value).catch(() => {});
    }
    const box = page.locator('button[role=checkbox][aria-checked=false]').filter({ visible: true });
    if (await box.count()) await box.first().click();
    if (!(await page.locator('#su-otp').count())) await submit(page);
    await page.waitForTimeout(700);
  }
  throw new Error(`setup: step ${n} not reached`);
}

/** A rewritten page with the gateway's error code (`<meta name="roedy-error">`, `#lib/errors.ts`). */
const gateway = (code) => ({ id: `gateway ${code}`, path: '/mail/', opts: mock(), gateway: code });

const SCENARIOS = [
  ...[1, 2, 3, 4, 5, 6, 7].map((n) => ({
    id: `setup step ${n}`,
    path: '/setup',
    opts: mock({ setup: true, routes: setupApi }),
    prepare: (page) => setupStep(page, n),
  })),
  {
    id: 'setup done',
    path: '/setup',
    opts: mock({ setup: true, routes: setupApi }),
    prepare: async (page) => {
      await setupStep(page, 7);
      await submit(page);
      await page.getByText('Welcome, Alex').waitFor();
    },
  },
  { id: 'login', path: '/login', opts: mock() },
  {
    id: 'login error',
    path: '/login',
    opts: mock({ routes: { 'POST auth/login': { status: 422, body: { error: 'invalid_credentials' } } } }),
    prepare: async (page) => {
      await fill(page, '#ln-id', 'alex');
      await fill(page, '#ln-pw', 'wrong');
      await submit(page);
      await page.locator('#ln-failed').waitFor();
    },
  },
  {
    id: 'login code',
    path: '/login',
    opts: mock({ routes: { 'POST auth/login': { body: { status: 'totp_required' } } } }),
    prepare: async (page) => {
      await fill(page, '#ln-id', 'alex@example.com');
      await fill(page, '#ln-pw', PASSWORD);
      await submit(page);
      await page.locator('#ln-otp').waitFor({ state: 'attached' });
    },
  },
  {
    id: 'login recovery code',
    path: '/login',
    opts: mock({ routes: { 'POST auth/login': { body: { status: 'totp_required' } } } }),
    prepare: async (page) => {
      await fill(page, '#ln-id', 'alex@example.com');
      await fill(page, '#ln-pw', PASSWORD);
      await submit(page);
      await page.locator('#ln-otp').waitFor({ state: 'attached' });
      await button(page, 'Use a recovery code');
      await page.locator('#ln-bc').waitFor({ state: 'attached' });
    },
  },
  {
    id: 'login hello',
    path: '/login',
    opts: mock({
      routes: {
        'POST auth/login': { body: { status: 'ok', user: { ...USER, display_name: USER.name } } },
        // Not signed in at first; then the session is held, so that "Hello" stays.
        'GET session': [{ status: 401, body: { error: 'unauthorized' } }, () => new Promise(() => {})],
      },
    }),
    prepare: async (page) => {
      await fill(page, '#ln-id', 'alex@example.com');
      await fill(page, '#ln-pw', PASSWORD);
      await submit(page);
      await page.getByText('Hello, Alex').waitFor();
    },
  },
  { id: 'login reauth', path: '/login?reauth=1&next=%2Fmail%2F', opts: mock({ signedIn: true }) },
  { id: 'forgot password', path: '/login/forgot', opts: mock() },
  {
    id: 'forgot password sent',
    path: '/login/forgot',
    opts: mock({ routes: { 'POST auth/password/forgot': { status: 202 } } }),
    prepare: async (page) => {
      await fill(page, '#ln-fg', 'alex@example.com');
      await submit(page);
      await page.getByText('Check your inbox').first().waitFor();
    },
  },
  { id: 'new password', path: '/login/reset/x', opts: mock() },
  {
    id: 'new password saved',
    path: '/login/reset/x',
    opts: mock({ routes: { 'POST auth/password/reset/x': { body: { status: 'ok', sessions_ended: 2 } } } }),
    prepare: async (page) => {
      await fill(page, '#ln-np', PASSWORD);
      await fill(page, '#ln-np2', PASSWORD);
      await submit(page);
      await page.getByText('Password changed').first().waitFor();
    },
  },
  {
    id: 'reset link invalid',
    path: '/login/reset/x',
    opts: mock({ routes: { 'POST auth/password/reset/x': { status: 422, body: { error: 'invalid_token' } } } }),
    prepare: async (page) => {
      await fill(page, '#ln-np', PASSWORD);
      await fill(page, '#ln-np2', PASSWORD);
      await submit(page);
      await page.getByText('Link expired or invalid').first().waitFor();
    },
  },
  ...invitation(),
  ...['valid', 'invalid', 'already_confirmed'].map((state) => ({
    id: `confirm email ${state}`,
    path: '/account/confirm-email/x',
    opts: mock({
      routes: {
        'POST auth/email/check': { body: { state, email: 'new@example.com', own_session: false } },
      },
    }),
    prepare: (page) => page.locator('h1').first().waitFor(),
  })),
  {
    id: 'confirm email confirmed',
    path: '/account/confirm-email/x',
    opts: mock({
      routes: {
        'POST auth/email/check': { body: { state: 'valid', email: 'new@example.com', own_session: false } },
        'POST auth/email/confirm': { body: { state: 'confirmed', email: 'new@example.com', own_session: false } },
      },
    }),
    prepare: async (page) => {
      await button(page, 'Confirm email address');
      await page.getByText('Email address confirmed').first().waitFor();
    },
  },
  { id: 'home', path: '/', opts: mock({ signedIn: true }) },
  { id: 'not found', path: '/does-not-exist/', opts: mock() },
  gateway('no_app_access'),
  gateway('app_unreachable'),
  gateway('app_not_ready'),
  gateway('app_timeout'),
  { id: 'offline', path: '/', opts: { offline: true } },
  // Account and administration: one module per page in `scenarios/`, each a function of the shared helpers.
  ...(await pageScenarios()),
];

/** Scenarios of `scenarios/*.mjs` (sorted by file name); each module's default export returns a list. */
async function pageScenarios() {
  const dir = path.join(import.meta.dirname, 'scenarios');
  if (!fs.existsSync(dir)) return [];
  const files = fs
    .readdirSync(dir)
    .filter((f) => f.endsWith('.mjs'))
    .sort();
  const helpers = { mock, fill, submit, button, USER, SESSION, CONFIG, BRANDING, ORG, ENROLL, CODES, PASSWORD };
  const lists = await Promise.all(files.map(async (f) => (await import(path.join(dir, f))).default(helpers)));
  return lists.flat();
}

function invitation() {
  const base = { 'GET invites/x': { body: INVITE } };
  const flow = {
    ...base,
    'POST invites/x/totp': { body: ENROLL },
    'POST invites/x/accept': { status: 201, body: { user: { ...USER, role: 'user' }, recovery_codes: CODES } },
  };
  const toStep = async (page, n) => {
    await page.getByRole('heading', { level: 1 }).first().waitFor();
    if (n < 2) return;
    await submit(page);
    await page.locator('#in-name').waitFor();
    if (n < 3) return;
    await fill(page, '#in-pw', PASSWORD);
    await fill(page, '#in-pw2', PASSWORD);
    await submit(page);
    await page.locator('#in-otp').waitFor({ state: 'attached' });
    if (n < 4) return;
    await page.locator('#in-otp').fill('123456');
    await page.getByRole('list', { name: 'Recovery codes' }).waitFor();
  };
  return [
    ...[1, 2, 3, 4].map((n) => ({
      id: `invitation step ${n}`,
      path: '/invite/x',
      opts: mock({ routes: flow }),
      prepare: (page) => toStep(page, n),
    })),
    {
      id: 'invitation welcome',
      path: '/invite/x',
      opts: mock({
        routes: {
          ...flow,
          'GET session': [{ status: 401, body: { error: 'unauthorized' } }, () => new Promise(() => {})],
        },
      }),
      prepare: async (page) => {
        await toStep(page, 4);
        await page.locator('button[role=checkbox]').first().click();
        await submit(page);
        await page.getByText('Welcome, Jordan').waitFor();
      },
    },
    {
      id: 'invitation expired',
      path: '/invite/x',
      opts: mock({
        routes: { 'GET invites/x': { body: { ...INVITE, expired: true } }, 'POST invites/x/request': { status: 202 } },
      }),
      prepare: (page) => page.getByText('Invitation expired').first().waitFor(),
    },
    {
      id: 'invitation requested',
      path: '/invite/x',
      opts: mock({
        routes: { 'GET invites/x': { body: { ...INVITE, expired: true } }, 'POST invites/x/request': { status: 202 } },
      }),
      prepare: async (page) => {
        await button(page, 'Request a new invitation');
        await page.getByText('Request sent').first().waitFor();
      },
    },
    {
      id: 'invitation load error',
      path: '/invite/x',
      opts: mock({ routes: { 'GET invites/x': { status: 500, body: { error: 'internal' } } } }),
      prepare: (page) => page.getByRole('heading', { level: 1 }).first().waitFor(),
    },
  ];
}

// --- Running ----------------------------------------------------------------

async function route(page, opts) {
  const used = new Map();
  await page.route('**/api/v1/**', async (req) => {
    if (opts.offline) return req.abort('internetdisconnected');
    const url = new URL(req.request().url());
    const key = `${req.request().method()} ${url.pathname.replace(/^\/api\/v1\//, '')}`;
    let answer = opts[key] ?? { status: 404, body: { error: 'not_found' } };
    if (Array.isArray(answer)) {
      const n = used.get(key) ?? 0;
      used.set(key, n + 1);
      answer = answer[Math.min(n, answer.length - 1)];
    }
    if (typeof answer === 'function') answer = await answer(req.request());
    await req.fulfill({
      status: answer.status ?? 200,
      contentType: 'application/json',
      body: answer.body === undefined ? '' : JSON.stringify(answer.body),
    });
  });
  // Branding files of the server: none in the audit. Matched on the path's start only: Vite serves the page
  // module of `/admin/branding` as `/src/routes/admin/branding/+page.svelte`, which must still load.
  await page.route(
    (url) => /^\/(branding\/|manifest\.webmanifest$)/.test(url.pathname),
    (req) => req.fulfill({ status: 404, body: '' }),
  );
}

async function runAxe(page) {
  if (!(await page.evaluate(() => !!window.axe))) await page.addScriptTag({ content: axe.source });
  return page.evaluate(async (tags) => {
    const r = await window.axe.run(document, { runOnly: { type: 'tag', values: tags } });
    return r.violations.map((v) => ({
      id: v.id,
      impact: v.impact,
      help: v.help,
      nodes: v.nodes.map((n) => ({ target: n.target.join(' '), summary: n.failureSummary })),
    }));
  }, AXE_TAGS);
}

async function check(browser, scenario, view, theme) {
  const context = await browser.newContext({
    viewport: VIEWS[view],
    colorScheme: theme,
    reducedMotion: 'reduce',
    locale: 'en-US',
  });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  await route(page, scenario.opts);
  if (scenario.gateway) {
    // The gateway serves the hub's page with the error code (milestone 4).
    await page.route(`${BASE}${scenario.path}`, async (req) => {
      const res = await req.fetch();
      const html = (await res.text()).replace(
        '<head>',
        `<head><meta name="roedy-error" content="${scenario.gateway}">`,
      );
      await req.fulfill({ response: res, body: html });
    });
  }
  try {
    await page.goto(BASE + scenario.path);
    await page.locator('h1').first().waitFor({ timeout: 15000 });
    if (scenario.prepare) await scenario.prepare(page);
    await page.waitForTimeout(400);
    const violations = await runAxe(page);
    const title = await page.title();
    const lang = await page.evaluate(() => document.documentElement.lang);
    const h1 = await page.locator('h1').count();
    const main = await page.locator('main, [role=main]').count();
    const problems = [];
    if (!title) problems.push('no <title>');
    if (!lang) problems.push('no lang on <html>');
    if (h1 !== 1) problems.push(`${h1} h1`);
    if (main !== 1) problems.push(`${main} main landmarks`);
    const name = `${scenario.id.replaceAll(/[^a-z0-9]+/gi, '-')}-${view}-${theme}.png`;
    if (shots) await page.screenshot({ path: path.join(PHOTOS, name), fullPage: true });
    return { id: scenario.id, view, theme, title, violations, problems, errors };
  } catch (error) {
    return {
      id: scenario.id,
      view,
      theme,
      violations: [],
      problems: [`failed: ${error.message.split('\n')[0]}`],
      errors,
    };
  } finally {
    await context.close();
  }
}

const chosen = SCENARIOS.filter((s) => !only.length || only.some((o) => s.id.includes(o)));
if (!chosen.length) {
  console.error(`--only ${only.join(',')} matches no scenario; known: ${SCENARIOS.map((s) => s.id).join(', ')}`);
  process.exit(2);
}
const browser = await chromium.launch();
const runs = chosen.flatMap((s) => Object.keys(VIEWS).flatMap((view) => THEMES.map((theme) => [s, view, theme])));
const results = [];
const queue = [...runs];
await Promise.all(
  Array.from({ length: 4 }, async () => {
    for (let run = queue.shift(); run; run = queue.shift()) {
      const result = await check(browser, ...run);
      results.push(result);
      const bad = result.violations.length + result.problems.length + result.errors.length;
      console.log(`${bad ? '✗' : '✓'} ${result.id} (${result.view}, ${result.theme})${bad ? '' : ''}`);
      for (const v of result.violations)
        console.log(`    ${v.id}: ${v.help} – ${v.nodes.map((n) => n.target).join(', ')}`);
      for (const p of result.problems) console.log(`    ${p}`);
      for (const e of result.errors) console.log(`    page error: ${e}`);
    }
  }),
);
await browser.close();

results.sort((a, b) => `${a.id}${a.view}${a.theme}`.localeCompare(`${b.id}${b.view}${b.theme}`));
fs.writeFileSync(path.join(OUT, 'report.json'), JSON.stringify(results, null, 2));
const failed = results.filter((r) => r.violations.length || r.problems.length || r.errors.length);
const md = [
  '# Accessibility audit of the hub',
  '',
  `${results.length} states (${SCENARIOS.length} scenarios × desktop/mobile × light/dark), axe tags ${AXE_TAGS.join(', ')}.`,
  '',
  failed.length ? `**${failed.length} with findings:**` : '**No findings.**',
  '',
  ...failed.flatMap((r) => [
    `- ${r.id} (${r.view}, ${r.theme})`,
    ...r.violations.map((v) => `  - ${v.id} (${v.impact}): ${v.help} – ${v.nodes.map((n) => n.target).join(', ')}`),
    ...r.problems.map((p) => `  - ${p}`),
    ...r.errors.map((e) => `  - page error: ${e}`),
  ]),
  '',
];
fs.writeFileSync(path.join(OUT, 'report.md'), md.join('\n'));
console.log(`\n${results.length} states, ${failed.length} with findings → ${path.join(OUT, 'report.md')}`);
// Any finding fails the run: a state that could not be audited proves nothing.
process.exit(failed.length ? 1 : 0);
