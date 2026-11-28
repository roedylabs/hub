import { fireEvent, render, within } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { session, type PublicConfig } from '#lib/state/session.svelte.ts';
import { BRAND0, ui } from '@roedy/ui';
import { page } from '#lib/test/page.svelte.ts';
import { sessionOf } from '#lib/test/session.ts';
import { setPhone } from '#lib/test/signed-in.ts';
import Layout from './+layout.svelte';

const nav = vi.hoisted(() => ({
  goto: vi.fn(() => Promise.resolve()),
  replace: vi.fn(),
  after: [] as ((navigation: unknown) => void)[],
}));
vi.mock('$app/navigation', () => ({
  goto: nav.goto,
  afterNavigate: (callback: (navigation: unknown) => void) => nav.after.push(callback),
}));
vi.mock('$app/state', () => ({ page }));

const CONFIG: PublicConfig = {
  name: 'Example Org',
  branding: BRAND0,
  org: { name: null, website: null, help: null, email: null, imprint: null, privacy: null, terms: null, source: null },
  setup_required: false,
};
const SESSION = sessionOf({ id: '01J', email: 'alex@example.com', name: 'Alex Example' });

const content = createRawSnippet(() => ({ render: () => '<p>Page content</p>' }));

afterEach(() => {
  setPhone(false);
  vi.unstubAllGlobals();
  nav.goto.mockClear();
  nav.replace.mockClear();
  nav.after.length = 0;
  vi.restoreAllMocks();
  session.config = null;
  session.user = null;
  session.current = null;
  session.status = 'idle';
  session.failure = null;
  prefs.reset();
  ui.reset();
  page.url = new URL('http://localhost/');
  page.status = 200;
  page.error = null;
  document.head.querySelector('meta[name="roedy-error"]')?.remove();
});

/** The layout at `path` with the public configuration and, if given, a session. */
function renderAt(path: string, signedIn: boolean, config: Partial<PublicConfig> = {}) {
  page.url = new URL(`http://localhost${path}`);
  const api = mockApi({
    'GET public/config': [{ body: { ...CONFIG, ...config } }],
    'GET session': [signedIn ? { body: SESSION } : { status: 401, body: { error: 'unauthorized' } }],
    'GET settings': [{ body: { version: 1, settings: { lang: 'en', mode: 'dark', accent: 'sage' } } }],
  });
  vi.stubGlobal('location', { ...window.location, origin: 'http://localhost', replace: nav.replace });
  return { view: render(Layout, { children: content }), ...api };
}

describe('root layout', () => {
  it('sends everything to the setup while no administrator exists', async () => {
    const { view, calls } = renderAt('/login', false, { setup_required: true });
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith('/setup', { replace: true }));
    expect(view.queryByText('Page content')).toBeNull();
    // Without an administrator nobody can be signed in.
    expect(calls.map((c) => c.path)).toEqual(['public/config']);
  });

  it('sends guests from the home page to the sign-in, without `next` for `/`', async () => {
    renderAt('/', false);
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith('/login', { replace: true }));
  });

  it('sends guests from the account pages to the sign-in with the whole address as `next`', async () => {
    renderAt('/account/security?tab=2#apps', false);
    await vi.waitFor(() =>
      expect(nav.goto).toHaveBeenCalledWith('/login?next=%2Faccount%2Fsecurity%3Ftab%3D2%23apps', { replace: true }),
    );
  });

  it.each(['/login', '/login/forgot', '/invite/tok', '/account/confirm-email/tok'])(
    'shows the guest page %s without sign-in',
    async (path) => {
      const { view } = renderAt(path, false);
      expect(await view.findByText('Page content')).toBeInTheDocument();
      expect(nav.goto).not.toHaveBeenCalled();
    },
  );

  it('leads the signed-in sign-in on to the hub page in `next`', async () => {
    renderAt('/login?next=%2Faccount', true);
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith('/account', { replace: true }));
    expect(nav.replace).not.toHaveBeenCalled();
  });

  it('leads the signed-in sign-in on to an app with a full page load', async () => {
    renderAt('/login?next=%2Fmail%2Fmailbox%2Finbox', true);
    await vi.waitFor(() => expect(nav.replace).toHaveBeenCalledWith('/mail/mailbox/inbox'));
    expect(nav.goto).not.toHaveBeenCalled();
  });

  it.each([
    'https://evil.example/',
    '//evil.example/',
    '/\\evil.example',
    '\\\\evil.example',
    'javascript:alert(1)',
    '/%0A/evil.example',
    '/a/..//evil.example',
  ])('never follows `next=%s` to another site', async (next) => {
    renderAt(`/login?next=${encodeURIComponent(next)}`, true);
    await vi.waitFor(() => expect(nav.goto.mock.calls.length + nav.replace.mock.calls.length).toBe(1));
    const [to] = (nav.goto.mock.calls[0] ?? nav.replace.mock.calls[0]) as unknown as [string];
    expect(to.startsWith('/') && !to.startsWith('//') && !to.includes('\\')).toBe(true);
    expect(new URL(to, 'http://localhost').origin).toBe('http://localhost');
  });

  it('keeps the sign-in for a fresh sign-in (`reauth=1`) of a signed-in account', async () => {
    const { view } = renderAt('/login?reauth=1&next=%2Fmail%2F', true);
    expect(await view.findByText('Page content')).toBeInTheDocument();
    expect(nav.goto).not.toHaveBeenCalled();
    expect(nav.replace).not.toHaveBeenCalled();
  });

  it('sends the signed-in account from the setup home', async () => {
    renderAt('/setup', true);
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith('/', { replace: true }));
  });

  it('loads the settings of a signed-in account and applies theme and language', async () => {
    const { view, calls } = renderAt('/', true);
    expect(await view.findByText('Page content')).toBeInTheDocument();
    await vi.waitFor(() => expect(document.documentElement.dataset.mode).toBe('dark'));
    expect(document.documentElement.dataset.accent).toBe('sage');
    expect(document.documentElement.lang).toBe('en');
    expect(calls.map((c) => c.path)).toEqual(['public/config', 'session', 'settings']);
    expect(document.title).toBe('Example Org');
    // The apps read theme and language before their first request.
    expect(JSON.parse(localStorage.getItem('roedy.hub.prefs') ?? '{}')).toEqual({ theme: 'dark', locale: 'en' });
  });

  it('shows "No connection" with "Try again" when the hub does not answer', async () => {
    page.url = new URL('http://localhost/');
    mockApi({});
    const view = render(Layout, { children: content });
    expect(await view.findByRole('heading', { level: 1, name: 'No connection' })).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
    expect(view.queryByText('Page content')).toBeNull();
  });

  it('loads the account’s settings after a successful "Try again" too', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    page.url = new URL('http://localhost/');
    const { calls } = mockApi({
      'GET public/config': [{ status: 500, body: { error: 'internal' } }, { body: CONFIG }],
      'GET session': [{ body: SESSION }],
      'GET settings': [{ body: { version: 1, settings: { lang: 'en', mode: 'dark', accent: null } } }],
    });
    const view = render(Layout, { children: content });
    await fireEvent.click(await view.findByRole('button', { name: 'Try again' }));
    await vi.advanceTimersByTimeAsync(900);
    expect(await view.findByText('Page content')).toBeInTheDocument();
    await vi.waitFor(() => expect(calls.map((c) => c.path)).toContain('settings'));
    await vi.waitFor(() => expect(prefs.values.mode).toBe('dark'));
    vi.useRealTimers();
  });

  it('shows the error the gateway put into the page without loading anything', () => {
    const meta = document.createElement('meta');
    meta.name = 'roedy-error';
    meta.content = 'no_app_access';
    document.head.append(meta);
    page.url = new URL('http://localhost/mail/');
    page.status = 404;
    page.error = { message: 'Not Found' };
    const { calls } = mockApi({});
    const error = createRawSnippet(() => ({ render: () => '<p>Router error</p>' }));
    const view = render(Layout, { children: error });
    expect(view.getByText('Router error')).toBeInTheDocument();
    expect(calls).toEqual([]);
    expect(nav.goto).not.toHaveBeenCalled();
  });
});

const heading = createRawSnippet(() => ({ render: () => '<h1>Page heading</h1>' }));

/** The layout signed in as `role` at `path` (an account or administration page), once the page shows. */
async function shellAt(path: string, role: 'admin' | 'user' = 'admin', phone = false) {
  setPhone(phone);
  page.url = new URL(`http://localhost${path}`);
  const api = mockApi({
    'GET public/config': [{ body: CONFIG }],
    'GET session': [{ body: sessionOf({ id: '01J', email: 'alex@example.com', name: 'Alex Example', role }) }],
    'GET settings': [{ body: { version: 1, settings: { lang: 'en', mode: 'light', accent: null } } }],
    'PUT settings': [{ body: { version: 2, settings: { lang: 'en', mode: 'dark', accent: null } } }],
  });
  vi.stubGlobal('location', { ...window.location, origin: 'http://localhost', replace: nav.replace });
  const view = render(Layout, { children: heading });
  await vi.waitFor(() => expect(view.container.querySelector('main')).not.toBeNull());
  return { view, ...api };
}

describe('shell of settings and administration (desktop)', () => {
  it('shows header, the area’s navigation and the page in one main', async () => {
    const { view } = await shellAt('/admin/users');
    expect(view.getByRole('banner')).toBeInTheDocument();
    const nav = view.getByRole('navigation', { name: 'Administration' });
    expect(within(nav).getByRole('button', { name: 'Users' })).toHaveAttribute('aria-current', 'page');
    expect(view.getAllByRole('main')).toHaveLength(1);
    expect(within(view.getByRole('main')).getByRole('heading', { level: 1, name: 'Page heading' })).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Administration' })).toHaveAttribute('aria-current', 'page');
    expect(view.getByRole('button', { name: 'Settings' })).not.toHaveAttribute('aria-current');
    await vi.waitFor(() => expect(document.title).toBe('Users – Example Org'));
  });

  it('opens the pages, the areas and the home page', async () => {
    const { view } = await shellAt('/account/security');
    await fireEvent.click(view.getByRole('button', { name: 'Appearance' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/account/appearance');
    // The open page again replaces its entry.
    await fireEvent.click(within(view.getByRole('navigation')).getByRole('button', { name: 'Security' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/account/security', { replace: true });
    await fireEvent.click(view.getByRole('button', { name: 'Administration' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/admin/users');
    await fireEvent.click(view.getByRole('button', { name: 'Back to home' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/');
    await fireEvent.click(view.getByRole('button', { name: 'Example Org' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/');
  });

  it.each([
    ['/account', '/account/profile'],
    ['/admin', '/admin/users'],
  ])('leads %s on to its first page', async (from, to) => {
    await shellAt(from);
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith(to, { replace: true }));
  });

  it('shows "Access denied" without the navigation to a user on an administration page', async () => {
    const { view } = await shellAt('/admin/users', 'user');
    expect(await view.findByRole('heading', { level: 1, name: 'Access denied' })).toBeInTheDocument();
    expect(view.queryByText('Page heading')).toBeNull();
    expect(view.queryByRole('navigation')).toBeNull();
    expect(view.getAllByRole('main')).toHaveLength(1);
    expect(view.queryByRole('button', { name: 'Administration' })).toBeNull();
    await vi.waitFor(() => expect(document.title).toBe('Access denied – Example Org'));
    // No way on to the first page either.
    nav.goto.mockClear();
    page.url = new URL('http://localhost/admin');
    await vi.waitFor(() => expect(view.getByRole('heading', { name: 'Access denied' })).toBeInTheDocument());
    expect(nav.goto).not.toHaveBeenCalled();
  });

  it('keeps the guest page "Confirm email" without the shell', async () => {
    page.url = new URL('http://localhost/account/confirm-email/tok');
    mockApi({
      'GET public/config': [{ body: CONFIG }],
      'GET session': [{ body: SESSION }],
      'GET settings': [{ body: { version: 1, settings: { lang: 'en', mode: 'light', accent: null } } }],
    });
    const view = render(Layout, { children: content });
    expect(await view.findByText('Page content')).toBeInTheDocument();
    expect(view.queryByRole('banner')).toBeNull();
    expect(view.queryByRole('main')).toBeNull();
  });

  it('puts the header above the home page, without navigation, with one main', async () => {
    page.url = new URL('http://localhost/');
    mockApi({
      'GET public/config': [{ body: CONFIG }],
      'GET session': [{ body: SESSION }],
      'GET settings': [{ body: { version: 1, settings: { lang: 'en', mode: 'light', accent: null } } }],
    });
    const view = render(Layout, { children: content });
    expect(await view.findByText('Page content')).toBeInTheDocument();
    expect(view.getByRole('banner')).toBeInTheDocument();
    expect(view.getAllByRole('main')).toHaveLength(1);
    expect(view.getByRole('main')).toHaveTextContent('Page content');
    expect(view.queryByRole('navigation')).toBeNull();
    // A user: settings, no administration.
    expect(view.getByRole('button', { name: 'Settings' })).toBeInTheDocument();
    expect(view.queryByRole('button', { name: 'Administration' })).toBeNull();
    // The account menu with "Sign out" opens from the avatar.
    await fireEvent.click(view.getByRole('button', { name: 'Alex Example' }));
    expect(view.getByRole('button', { name: 'Sign out' })).toBeInTheDocument();
  });

  it('opens the account menu below the avatar; Escape and a click outside close it', async () => {
    const { view } = await shellAt('/account/profile');
    const avatar = view.getByRole('button', { name: 'Alex Example' });
    await fireEvent.click(avatar);
    const menu = view.getByRole('group', { name: 'Alex Example' });
    expect(avatar).toHaveAttribute('aria-expanded', 'true');
    expect(menu).toHaveTextContent('alex@example.com');
    const manage = within(menu).getByRole('button', { name: 'Manage account' });
    manage.focus();
    await fireEvent.keyDown(manage, { key: 'Escape' });
    expect(view.queryByRole('group', { name: 'Alex Example' })).toBeNull();
    expect(avatar).toHaveFocus();
    // A click inside keeps it open, one outside closes it.
    await fireEvent.click(avatar);
    await fireEvent.mouseDown(view.getByRole('group', { name: 'Alex Example' }));
    expect(view.getByRole('group', { name: 'Alex Example' })).toBeInTheDocument();
    await fireEvent.mouseDown(view.getByRole('main'));
    expect(view.queryByRole('group', { name: 'Alex Example' })).toBeNull();
    // "Manage account" leads to the profile, the colour scheme applies at once.
    await vi.waitFor(() => expect(prefs.remote).toBe(true));
    await fireEvent.click(avatar);
    await fireEvent.click(view.getByRole('radio', { name: 'Dark' }));
    expect(prefs.values.mode).toBe('dark');
    await prefs.settled();
    expect(prefs.values.mode).toBe('dark');
    await fireEvent.click(view.getByRole('button', { name: 'Manage account' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/account/profile', { replace: true });
    expect(view.queryByRole('group', { name: 'Alex Example' })).toBeNull();
  });

  it('asks above the page; Escape cancels, "Confirm" runs', async () => {
    const { view } = await shellAt('/admin/users');
    const run = vi.fn();
    ui.ask({ title: 'Remove Jordan?', text: 'Jordan can no longer sign in.', ok: 'Remove', danger: true, run });
    const dialog = await view.findByRole('alertdialog', { name: 'Remove Jordan?' });
    expect(dialog).toHaveAccessibleDescription('Jordan can no longer sign in.');
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(view.queryByRole('alertdialog')).toBeNull();
    expect(run).not.toHaveBeenCalled();
    ui.ask({ title: 'Remove Jordan?', text: '', ok: 'Remove', run });
    await fireEvent.click(await view.findByRole('button', { name: 'Remove' }));
    expect(run).toHaveBeenCalledOnce();
  });

  it('shows a page’s dialog above the shell and closes it with Escape', async () => {
    const { view } = await shellAt('/account/appearance');
    const close = vi.fn();
    const dialog = createRawSnippet((mobile: () => boolean) => ({
      render: () => `<div role="dialog" aria-label="More colors">${mobile() ? 'sheet' : 'popover'}</div>`,
    }));
    ui.pageDialog = () => ({ title: 'More colors', close, content: dialog, bare: true });
    expect(await view.findByRole('dialog', { name: 'More colors' })).toHaveTextContent('popover');
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(close).toHaveBeenCalledOnce();
  });
});

describe('shell of settings and administration (mobile)', () => {
  it('shows the area’s list on its start with the brand (home) and the avatar (account menu)', async () => {
    const { view } = await shellAt('/account', 'admin', true);
    expect(view.getByRole('heading', { level: 1, name: 'Settings' })).toBeInTheDocument();
    expect(view.queryByText('Page heading')).toBeNull();
    expect(view.getAllByRole('main')).toHaveLength(1);
    // No redirect to the first page on mobile.
    expect(nav.goto).not.toHaveBeenCalledWith('/account/profile', { replace: true });
    await fireEvent.click(view.getByRole('button', { name: 'Security' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/account/security');
    await fireEvent.click(view.getByRole('button', { name: 'Example Org' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/');
    await vi.waitFor(() => expect(document.title).toBe('Settings – Example Org'));
    // The account menu with sign-out and, for administrators, the administration.
    await fireEvent.click(view.getByRole('button', { name: /Alex Example/ }));
    const sheet = await view.findByRole('dialog', { name: 'Alex Example' });
    expect(within(sheet).getByRole('button', { name: 'Sign out' })).toBeInTheDocument();
    await fireEvent.click(within(sheet).getByRole('button', { name: 'Administration' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/admin/users');
  });

  it('offers no administration in the account menu to a user', async () => {
    const { view } = await shellAt('/account', 'user', true);
    await fireEvent.click(view.getByRole('button', { name: /Alex Example/ }));
    const sheet = await view.findByRole('dialog', { name: 'Alex Example' });
    expect(within(sheet).queryByRole('button', { name: 'Administration' })).toBeNull();
  });

  it('shows the page with "Back" to its area’s list', async () => {
    const { view } = await shellAt('/admin/audit-log', 'admin', true);
    expect(within(view.getByRole('main')).getByText('Page heading')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Administration' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/admin');
  });

  it('shows "Access denied" with "Back to home" to a user', async () => {
    const { view } = await shellAt('/admin', 'user', true);
    expect(await view.findByRole('heading', { level: 1, name: 'Access denied' })).toBeInTheDocument();
    expect(view.getAllByRole('main')).toHaveLength(1);
    await fireEvent.click(view.getByRole('button', { name: 'Back to home' }));
    expect(nav.goto).toHaveBeenLastCalledWith('/');
  });

  it('shows the account menu and a page’s select list as sheets', async () => {
    const { view } = await shellAt('/admin/users', 'admin', true);
    ui.menu = 'account';
    const sheet = await view.findByRole('dialog', { name: 'Alex Example' });
    expect(within(sheet).getByRole('button', { name: 'Sign out' })).toBeInTheDocument();
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(view.queryByRole('dialog')).toBeNull();
    const choose = vi.fn();
    ui.pick = () => ({
      title: 'Role',
      opts: [{ label: 'Administrator', sub: '', hasSub: false, on: false, fw: 600, onClick: choose }],
    });
    ui.menu = 'pick';
    const pick = await view.findByRole('dialog', { name: 'Role' });
    await fireEvent.click(within(pick).getByRole('option', { name: 'Administrator' }));
    expect(choose).toHaveBeenCalled();
    // Another page closes the sheet.
    page.url = new URL('http://localhost/admin/branding');
    await vi.waitFor(() => expect(view.queryByRole('dialog')).toBeNull());
  });
});

/** A navigation as SvelteKit reports it to `afterNavigate`: from `from` to the current address. */
function arrived(from: string, type = 'link', delta?: number) {
  for (const callback of nav.after)
    callback({ from: { url: new URL(`http://localhost${from}`) }, to: null, type, delta });
}

describe('shell across the breakpoint', () => {
  it('keeps the page mounted when the window crosses between desktop and mobile', async () => {
    const { view } = await shellAt('/account/profile');
    const pageHeading = view.getByRole('heading', { level: 1, name: 'Page heading' });
    expect(view.getByRole('navigation', { name: 'Settings' })).toBeInTheDocument();
    setPhone(true);
    // Mobile: "Back" to the area's list instead of header and navigation.
    await vi.waitFor(() => expect(view.queryByRole('navigation')).toBeNull());
    expect(view.getByRole('button', { name: 'Settings' })).toBeInTheDocument();
    expect(view.getAllByRole('main')).toHaveLength(1);
    expect(view.getByRole('heading', { level: 1, name: 'Page heading' })).toBe(pageHeading);
    setPhone(false);
    await vi.waitFor(() => expect(view.getByRole('navigation', { name: 'Settings' })).toBeInTheDocument());
    expect(view.getByRole('heading', { level: 1, name: 'Page heading' })).toBe(pageHeading);
  });
});

describe('"Back" of the shell', () => {
  it('goes back in the history only to the entry before (without the Navigation API)', async () => {
    const back = vi.spyOn(history, 'back').mockImplementation(() => {});
    const { view } = await shellAt('/account/profile', 'admin', true);
    arrived('/account');
    await fireEvent.click(view.getByRole('button', { name: 'Settings' }));
    expect(back).toHaveBeenCalledOnce();
    // `/` → `/account/profile` → `/account`, then browser Back: `/account` now lies ahead, not before.
    nav.goto.mockClear();
    arrived('/account', 'popstate', -1);
    await fireEvent.click(view.getByRole('button', { name: 'Settings' }));
    expect(back).toHaveBeenCalledOnce();
    expect(nav.goto).toHaveBeenLastCalledWith('/account');
  });

  it('does not take a replaced entry for the one before (without the Navigation API)', async () => {
    const browser = { length: 4, back: vi.fn() };
    vi.stubGlobal('history', browser);
    const { view } = await shellAt('/account/profile', 'admin', true);
    // `/` → `/account` (a new entry) → `/account/profile` in its place (the desktop's redirect).
    arrived('/');
    arrived('/account', 'goto');
    await fireEvent.click(view.getByRole('button', { name: 'Settings' }));
    expect(browser.back).not.toHaveBeenCalled();
    expect(nav.goto).toHaveBeenLastCalledWith('/account');
    // Two entries back, then on to a new one: the two ahead are dropped, the history shrinks, the entry is new.
    arrived('/', 'popstate', -2);
    browser.length = 3;
    arrived('/account');
    await fireEvent.click(view.getByRole('button', { name: 'Settings' }));
    expect(browser.back).toHaveBeenCalledOnce();
  });

  it('checks the entry before with the Navigation API', async () => {
    const back = vi.spyOn(history, 'back').mockImplementation(() => {});
    const entries = (...paths: string[]) => paths.map((p) => ({ url: `http://localhost${p}` }));
    const navigation = {
      canGoBack: true,
      currentEntry: { index: 1 },
      entries: () => entries('/', '/account/profile', '/account'),
    };
    vi.stubGlobal('navigation', navigation);
    const { view } = await shellAt('/account/profile', 'admin', true);
    arrived('/account', 'popstate', -1);
    await fireEvent.click(view.getByRole('button', { name: 'Settings' }));
    expect(back).not.toHaveBeenCalled();
    expect(nav.goto).toHaveBeenLastCalledWith('/account');
    navigation.currentEntry = { index: 2 };
    navigation.entries = () => entries('/', '/account', '/account/profile');
    await fireEvent.click(view.getByRole('button', { name: 'Settings' }));
    expect(back).toHaveBeenCalledOnce();
  });
});
