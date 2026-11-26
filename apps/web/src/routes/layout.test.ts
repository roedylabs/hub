import { render } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { BRAND0, session, type PublicConfig } from '#lib/state/session.svelte.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { page } from '#lib/test/page.svelte.ts';
import { sessionOf } from '#lib/test/session.ts';
import Layout from './+layout.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()), replace: vi.fn() }));
vi.mock('$app/navigation', () => ({ goto: nav.goto, afterNavigate: () => {} }));
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
  vi.unstubAllGlobals();
  nav.goto.mockClear();
  nav.replace.mockClear();
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
