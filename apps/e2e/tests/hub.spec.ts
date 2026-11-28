// The hub and the example app end to end, over HTTPS as deployed: plain HTTP
// redirects with HSTS → setup with the mandatory authenticator app → sign-in
// with password and code → pairing the example app from Administration › Apps
// → its tile on the home page opens it through the gateway with the identity
// → SSE and WebSocket through the browser → a user without access gets the
// 403 page → every page state in desktop and mobile, light and dark → sign-out
// closes the streams and clears the site data. axe checks every page state.
// The tests build on each other (serial) and share the run's hub.
import { expect, test, type Locator, type Page } from '@playwright/test';
import { env } from '../env.ts';
import {
  Authenticator,
  LOOKS,
  closeAll,
  expectAccessible,
  need,
  newContext,
  newPage,
  pairExample,
  password,
  totp,
  type Session,
} from './helpers.ts';

test.describe.configure({ mode: 'serial' });

/** The administrator of the setup; `app` is the authenticator app linked there (its codes are valid once). */
const admin: { name: string; email: string; password: string; app?: Authenticator; state?: Session } = {
  name: 'Alex Admin',
  email: 'admin@example.com',
  password: password(),
};
/** The invited user without access to the example app. */
const member: { name: string; email: string; password: string; link?: string; state?: Session } = {
  name: 'Sam Member',
  email: 'member@example.com',
  password: password(),
};

// Also the contexts of a failed test: otherwise their pages would keep running and report errors in the next one.
test.afterEach(async ({ browser }) => {
  expect(await closeAll(browser), 'page errors').toEqual([]);
});

/** The visible button `name`: in the card on desktop, in the bottom bar on mobile. */
function button(page: Page, name: string | RegExp): Locator {
  return page
    .getByRole('button', { name, exact: typeof name === 'string' })
    .filter({ visible: true })
    .last();
}

/** A page state: the level-1 heading is shown and axe finds nothing. */
async function state(page: Page, heading: string | RegExp, what = String(heading)): Promise<void> {
  await expect(page.getByRole('heading', { level: 1, name: heading })).toBeVisible();
  await expectAccessible(page, what);
}

/** The heading of the authenticator app step (with its soft hyphen). */
const MFA = /^Multi-factor authen­?tication$/;

test('plain HTTP redirects to HTTPS, which sends HSTS', async ({ browser, request }) => {
  const plain = await request.get(`${env.httpUrl}/login?next=%2Fexample%2F`, { maxRedirects: 0 });
  expect(plain.status()).toBe(308);
  expect(plain.headers()['location']).toBe(`${env.url}/login?next=%2Fexample%2F`);
  const secure = await request.get(`${env.url}/`);
  expect(secure.headers()['strict-transport-security']).toMatch(/^max-age=31536000\b/);

  // The browser follows it and trusts the hub's certificate.
  const page = await newPage(await newContext(browser, { view: 'desktop', theme: 'light' }));
  const response = await page.goto(`${env.httpUrl}/`);
  expect(response?.url()).toMatch(new RegExp(`^${env.url}/`));
  expect(response?.request().redirectedFrom()?.url()).toBe(`${env.httpUrl}/`);
});

test('setup with the mandatory authenticator app', async ({ browser }) => {
  const page = await newPage(await newContext(browser, { view: 'desktop', theme: 'light' }));
  await page.goto('/');
  await expect(page).toHaveURL(/\/setup$/);
  await state(page, 'Welcome to Roedy Hub');
  await button(page, 'Get started').click();

  await state(page, 'Set up Roedy Hub', 'setup token');
  await page.locator('#su-token').fill(env.setupToken);
  await button(page, 'Continue').click();

  await state(page, 'Create administrator');
  await page.locator('#su-name').fill(admin.name);
  await page.locator('#su-email').fill(admin.email);
  await button(page, 'Continue').click();

  await state(page, 'Set password');
  await page.locator('#su-pw').fill(admin.password);
  await page.locator('#su-pw2').fill(admin.password);
  await button(page, 'Continue').click();

  // The authenticator app: the key shown next to the QR code, a code from it.
  await state(page, MFA, 'authenticator app');
  const key = (await page.locator('.key-value').innerText()).replace(/\s/g, '');
  expect(key).toMatch(/^[A-Z2-7]{16,}$/);
  await page.locator('#su-otp').fill(totp(key));
  admin.app = new Authenticator(key);

  await state(page, 'Recovery codes');
  await expect(page.getByRole('list', { name: 'Recovery codes' }).getByRole('listitem')).toHaveCount(10);
  await page.getByRole('checkbox', { name: 'I’ve saved the codes in a safe place' }).click();
  await button(page, 'Continue').click();

  await state(page, 'All set?');
  await button(page, 'Finish setup').click();
  await state(page, 'Welcome, Alex');

  // Signed in right away: the home page without apps yet.
  await expect(page).toHaveURL(`${env.url}/`);
  await state(page, 'Hello, Alex', 'home page without apps');
  await expect(page.getByText('Add your first app')).toBeVisible();
});

test('sign-in with password and authenticator app', async ({ browser }) => {
  const app = need(admin.app, 'authenticator app');
  const context = await newContext(browser, { view: 'mobile', theme: 'dark' });
  const page = await newPage(context);
  await page.goto('/admin/apps');
  await expect(page).toHaveURL(/\/login\?next=%2Fadmin%2Fapps$/);
  await state(page, 'Welcome back', 'sign-in');
  await page.locator('#ln-id').fill(admin.email);
  await page.locator('#ln-pw').fill(admin.password);
  await button(page, 'Sign in').click();

  await state(page, MFA, 'sign-in code');
  await page.locator('#ln-otp').fill(await app.next());
  // Back where the sign-in started.
  await expect(page).toHaveURL(`${env.url}/admin/apps`);
  await state(page, 'Apps', 'apps without any');
  admin.state = await context.storageState();
});

test('pairing the example app from Administration › Apps', async ({ browser }) => {
  const page = await newPage(
    await newContext(browser, { view: 'desktop', theme: 'dark' }, need(admin.state, 'session')),
  );
  await page.goto('/admin/apps');
  await state(page, 'Apps', 'apps without any');
  await button(page, 'Add app').click();

  const form = page.getByRole('form', { name: 'Add app' });
  await expect(form).toBeVisible();
  await expectAccessible(page, 'add app');
  await form.getByLabel('Address', { exact: true }).fill('example');
  await form.getByLabel('Internal address').fill(env.exampleInternal);
  await form.getByRole('button', { name: 'Add', exact: true }).click();

  // The code is shown once; the example app pairs with it at its start.
  const shown = page.getByRole('region', { name: 'Pairing code for example' });
  await expect(shown).toBeVisible();
  await expectAccessible(page, 'pairing code');
  const code = (await shown.getByRole('code').innerText()).replace(/\s/g, '');
  expect(code).toMatch(/^rhp_[0-9A-Z]+$/);
  await expect(page.getByRole('button', { name: /^example \/example\/ Waiting for pairing$/ })).toBeVisible();
  await pairExample(code);

  // Paired with its manifest's name, and reachable.
  await expect(async () => {
    await page.reload();
    await expect(page.getByRole('button', { name: /^Example \/example\/ · 1 user Online$/ })).toBeVisible({
      timeout: 1000,
    });
  }).toPass({ timeout: 30_000 });
  await state(page, 'Apps', 'apps with the example');

  await page.getByRole('button', { name: /^Example / }).click();
  await expect(page).toHaveURL(/\/admin\/apps\/example$/);
  await state(page, 'Example', 'app details');
  await expect(page.getByText('roedy.example')).toBeVisible();
});

test('the tile on the home page opens the app with the identity', async ({ browser }) => {
  const page = await newPage(
    await newContext(browser, { view: 'desktop', theme: 'light' }, need(admin.state, 'session')),
  );
  await page.goto('/');
  await state(page, 'Hello, Alex', 'home page with the example');
  const tile = page.getByRole('link', { name: 'Example' });
  await expect(tile).toHaveAttribute('href', '/example/');
  await tile.click();

  // Through the gateway: the example verified the identity header the hub signed.
  await expect(page).toHaveURL(`${env.url}/example/`);
  await state(page, 'Example', 'example app');
  await expect(page.getByText(`Signed in as ${admin.name}`)).toBeVisible();
  const identity = await page.evaluate(async () => (await fetch('/example/whoami')).json() as Promise<unknown>);
  expect(identity).toMatchObject({
    iss: env.url,
    aud: 'example',
    email: admin.email,
    name: admin.name,
    role: 'admin',
    app_role: 'admin',
  });
  // Its link back leads to the hub's home page.
  await page.getByRole('link', { name: 'Home' }).click();
  await state(page, 'Hello, Alex', 'home page after the app');
});

test('the app switcher in the example app, under its strict CSP', async ({ browser, request }) => {
  const script = await request.get(`${env.url}/api/v1/app-switcher.js`);
  expect(script.status()).toBe(200);
  expect(script.headers()['content-type']).toBe('text/javascript; charset=utf-8');
  expect(script.headers()['cache-control']).toBe('no-cache');

  const page = await newPage(
    await newContext(browser, { view: 'desktop', theme: 'dark' }, need(admin.state, 'session')),
  );
  const blocked: string[] = [];
  page.on('console', (message) => {
    if (/content security policy/i.test(message.text())) blocked.push(message.text());
  });
  await page.goto('/example/');
  await state(page, 'Example', 'example app with the switcher');

  // The menu button opens with the keyboard, on the first item: the example, marked as the current app.
  const button = page.getByRole('button', { name: 'Apps' });
  await expect(button).toHaveAttribute('aria-expanded', 'false');
  await button.focus();
  await page.keyboard.press('Enter');
  const menu = page.getByRole('menu', { name: 'Apps' });
  await expect(menu).toBeVisible();
  await expect(button).toHaveAttribute('aria-expanded', 'true');
  const items = menu.getByRole('menuitem');
  await expect(items).toHaveText(['Example', 'Home', 'Manage account', 'Administration']);
  const example = menu.getByRole('menuitem', { name: 'Example' });
  await expect(example).toBeFocused();
  await expect(example).toHaveAttribute('aria-current', 'page');
  await expect(example).toHaveAttribute('href', '/example/');
  // Its icon from the hub, allowed by `img-src 'self'`.
  await expect
    .poll(() => example.locator('img').evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth > 0))
    .toBe(true);

  // Styled by its constructed style sheet (no `style-src` on the page), in the system's dark mode.
  const pop = page.locator('roedy-app-switcher .pop');
  await expect(pop).toHaveCSS('border-radius', '28px');
  await expect(pop).toHaveCSS('background-color', 'rgb(50, 43, 37)');
  await expectAccessible(page, 'app switcher open');

  await page.keyboard.press('End');
  await expect(menu.getByRole('menuitem', { name: 'Administration' })).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(example).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(button).toBeFocused();

  // A click outside closes it; "Home" leads to the hub.
  await button.click();
  await expect(menu).toBeVisible();
  await page.getByRole('heading', { name: 'Example' }).click();
  await expect(menu).toBeHidden();
  await button.click();
  await menu.getByRole('menuitem', { name: 'Home' }).click();
  await state(page, 'Hello, Alex', 'home page after the switcher');
  expect(blocked).toEqual([]);
});

test('SSE and WebSocket through the browser', async ({ browser }) => {
  const page = await newPage(
    await newContext(browser, { view: 'mobile', theme: 'light' }, need(admin.state, 'session')),
  );
  await page.goto('/example/');
  await expect(page.getByText(`Signed in as ${admin.name}`)).toBeVisible();

  /** The `tick` events of `/example/sse?<query>` until `count` arrived (or the stream fails). */
  const ticks = (query: string, count: number) =>
    page.evaluate(
      ([query, count]) =>
        new Promise<string[]>((resolve, reject) => {
          const source = new EventSource(`/example/sse?${query}`);
          const data: string[] = [];
          source.addEventListener('tick', (e) => {
            data.push(e.data as string);
            if (data.length === count) {
              source.close();
              resolve(data);
            }
          });
          source.onerror = () => {
            source.close();
            reject(new Error(`EventSource failed after ${data.length} events`));
          };
        }),
      [query, count] as const,
    );
  // Not buffered: the first event of an endless stream (the next one would follow after a minute) arrives at once.
  const started = Date.now();
  expect(await ticks('count=0&interval_ms=60000', 1)).toEqual(['tick 0']);
  expect(Date.now() - started).toBeLessThan(10_000);
  // A finite stream arrives complete and in order.
  expect(await ticks('count=3&interval_ms=100', 3)).toEqual(['tick 0', 'tick 1', 'tick 2']);

  // WebSocket echo through the gateway, text and binary, and a clean close.
  const ws = await page.evaluate(
    () =>
      new Promise<{ text: string; binary: number[]; code: number }>((resolve, reject) => {
        const socket = new WebSocket(`wss://${location.host}/example/ws`);
        socket.binaryType = 'arraybuffer';
        const got: { text?: string; binary?: number[] } = {};
        socket.onopen = () => {
          socket.send('hello through the hub');
          socket.send(new Uint8Array([1, 2, 3, 250]));
        };
        socket.onmessage = (e) => {
          if (typeof e.data === 'string') got.text = e.data;
          else got.binary = [...new Uint8Array(e.data as ArrayBuffer)];
          if (got.text !== undefined && got.binary) socket.close(1000, 'done');
        };
        socket.onclose = (e) => resolve({ text: got.text ?? '', binary: got.binary ?? [], code: e.code });
        socket.onerror = () => reject(new Error('WebSocket failed'));
      }),
  );
  expect(ws).toEqual({ text: 'hello through the hub', binary: [1, 2, 3, 250], code: 1000 });
});

test('a user without access gets the 403 page', async ({ browser }) => {
  // The administrator invites without access to the example (no system mail: the link is copied).
  const adminPage = await newPage(
    await newContext(browser, { view: 'desktop', theme: 'light' }, need(admin.state, 'session')),
  );
  await adminPage.goto('/admin/users');
  await state(adminPage, 'Users');
  await button(adminPage, 'Invite').click();
  await adminPage.getByLabel('Email addresses').fill(member.email);
  await adminPage.getByLabel('Email addresses').press('Enter');
  await adminPage.getByLabel('Name', { exact: true }).fill(member.name);
  await adminPage
    .getByRole('radiogroup', { name: 'Access to Example' })
    .getByRole('radio', { name: 'No access' })
    .click();
  await expectAccessible(adminPage, 'invite form');
  const created = adminPage.waitForResponse(
    (r) => r.url().endsWith('/api/v1/invites') && r.request().method() === 'POST',
  );
  await button(adminPage, 'Create invitation').click();
  const invite = ((await (await created).json()) as { invites: { id: string }[] }).invites[0];
  const link = await adminPage.evaluate(
    async (id) => {
      const r = await fetch(`/api/v1/invites/${id}/link`, { headers: { 'X-Requested-With': 'fetch' } });
      return ((await r.json()) as { url: string }).url;
    },
    need(invite?.id, 'invitation'),
  );
  expect(link).toMatch(new RegExp(`^${env.url}/invite/`));
  member.link = link;

  // The invitation on a phone, without an authenticator app ("Set up later").
  const context = await newContext(browser, { view: 'mobile', theme: 'light' });
  const page = await newPage(context);
  await page.goto(link);
  await state(page, 'Welcome to Roedy Hub', 'invitation');
  await button(page, /^(Accept invitation|Continue)$/).click();
  await expect(page.locator('#in-name')).toBeVisible();
  await expectAccessible(page, 'invitation name and password');
  await page.locator('#in-name').fill(member.name);
  await page.locator('#in-pw').fill(member.password);
  await page.locator('#in-pw2').fill(member.password);
  await button(page, /^(Continue|Activate account)$/).click();
  await expect(page.locator('#in-otp')).toBeAttached();
  await expectAccessible(page, 'invitation authenticator app');
  await button(page, 'Set up later').click();
  await state(page, /^Welcome, Sam/, 'invitation welcome');

  // Signed in without apps; the example answers with the hub's 403 page.
  await page.goto('/');
  await state(page, 'Hello, Sam', 'home page without access');
  await expect(page.getByText('No apps yet')).toBeVisible();
  const denied = await page.goto('/example/');
  expect(denied?.status()).toBe(403);
  await state(page, 'No access to this app');
  member.state = await context.storageState();
});

// Each page state once more in every view and theme (the flows above run in one each).
for (const look of LOOKS) {
  test(`page states on ${look.view} in ${look.theme}`, async ({ browser }) => {
    const guest = await newPage(await newContext(browser, look));
    await guest.goto('/login');
    await state(guest, 'Welcome back', `sign-in ${look.view} ${look.theme}`);

    const page = await newPage(await newContext(browser, look, need(admin.state, 'session')));
    for (const [path, heading] of [
      ['/', 'Hello, Alex'],
      ['/admin/apps', 'Apps'],
      ['/admin/apps/example', 'Example'],
      ['/admin/users', 'Users'],
      ['/example/', 'Example'],
    ] as const) {
      await page.goto(path);
      await state(page, heading, `${path} ${look.view} ${look.theme}`);
    }

    const denied = await newPage(await newContext(browser, look, need(member.state, 'member session')));
    await denied.goto('/example/');
    await state(denied, 'No access to this app', `403 ${look.view} ${look.theme}`);
  });
}

test('sign-out closes the streams and clears the site data', async ({ browser }) => {
  const context = await newContext(browser, { view: 'desktop', theme: 'light' }, need(admin.state, 'session'));
  // An open SSE stream and WebSocket in the app …
  const app = await newPage(context);
  await app.goto('/example/');
  await expect(app.getByText(`Signed in as ${admin.name}`)).toBeVisible();
  await app.evaluate(
    () =>
      new Promise<void>((resolve) => {
        // The phase in which each stream ended: only an end after the sign-out started counts.
        const w = window as unknown as { phase: string; ended: { sse: string; ws: string } };
        w.phase = 'before sign-out';
        w.ended = { sse: 'open', ws: 'open' };
        // An endless stream; the first end counts (EventSource would connect again, then without identity).
        const sse = new EventSource('/example/sse?count=0&interval_ms=500');
        sse.onerror = () => {
          w.ended.sse = w.phase;
          sse.close();
        };
        const ws = new WebSocket(`wss://${location.host}/example/ws`);
        ws.onclose = () => (w.ended.ws = w.phase);
        let open = 0;
        const opened = () => ++open === 2 && resolve();
        sse.addEventListener('tick', opened, { once: true });
        ws.onopen = opened;
      }),
  );

  // … then the sign-out on the hub's page in another tab.
  const hub = await newPage(context);
  await hub.goto('/');
  await state(hub, 'Hello, Alex');
  await hub.getByRole('banner').getByRole('button', { name: admin.name }).click();
  await expectAccessible(hub, 'account menu');
  // Both streams still open right before the sign-out.
  expect(await app.evaluate(() => (window as unknown as { ended: unknown }).ended)).toEqual({
    sse: 'open',
    ws: 'open',
  });
  await app.evaluate(() => ((window as unknown as { phase: string }).phase = 'sign-out'));
  const loggedOut = hub.waitForResponse((r) => r.url().endsWith('/api/v1/auth/logout'));
  await hub.getByRole('button', { name: 'Sign out' }).click();
  const response = await loggedOut;
  expect(response.status()).toBe(204);
  // `headers()` leaves out security-related headers.
  expect((await response.allHeaders())['clear-site-data']).toBe('"cache", "storage"');
  await expect(hub).toHaveURL(/\/login$/);
  await state(hub, 'Welcome back', 'sign-in after sign-out');
  expect((await context.cookies()).map((c) => c.name)).not.toContain('__Host-roedy_hub_session');

  // The hub ended the session's stream and WebSocket to the app at once.
  await expect
    .poll(() => app.evaluate(() => (window as unknown as { ended: unknown }).ended), { timeout: 5000 })
    .toEqual({ sse: 'sign-out', ws: 'sign-out' });
  // The app gets no identity any more: its page is public, `whoami` is not (contract § 5.1).
  await app.reload();
  await expect(app.getByText('Not signed in')).toBeVisible();
  expect(await app.evaluate(async () => (await fetch('/example/whoami')).status)).toBe(401);
});
