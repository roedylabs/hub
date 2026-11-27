import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AdminApp, HomeApp } from '#lib/apps.ts';
import { HomePage, homeValues } from '#lib/home.svelte.ts';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { resetSignedIn, signIn } from '#lib/test/signed-in.ts';

const MAIL: HomeApp = {
  slug: 'mail',
  name: 'Mail',
  description: 'Email for the team',
  url: '/mail/',
  icon: '/api/v1/apps/mail/icon.svg?v=0123456789abcdef',
  status: 'online',
};
const NOTES: HomeApp = { ...MAIL, slug: 'notes', name: 'Notes', description: 'Shared notes', url: '/notes/' };

const admin = (slug: string, state: 'pending' | 'paired', name?: string) =>
  ({ slug, state, name: name ? { en: name, de: name } : null }) as unknown as AdminApp;

const ctx = { openApps: vi.fn() };

afterEach(() => {
  resetSignedIn();
  ctx.openApps.mockClear();
});

async function started(routes: Parameters<typeof mockApi>[0]) {
  const api = mockApi(routes);
  const p = new HomePage();
  p.start();
  await vi.waitFor(() => expect(p.status).not.toBe('loading'));
  return { p, v: () => homeValues(p, ctx), ...api };
}

describe('home page', () => {
  it('shows the user’s apps as tiles with their offline state', async () => {
    signIn('user');
    const { v, calls } = await started({ 'GET apps': [{ body: { apps: [MAIL, { ...NOTES, status: 'offline' }] } }] });
    expect(v().greeting).toBe('Hello, Alex');
    expect(v().lead).toBe('Your apps at Example Org');
    expect(v().tiles.map((t) => [t.name, t.url, t.offline])).toEqual([
      ['Mail', '/mail/', false],
      ['Notes', '/notes/', true],
    ]);
    expect(v()).toMatchObject({ empty: false, pendingHint: null, emptyAction: null });
    // A user never asks the administration.
    expect(calls.map((c) => c.path)).toEqual(['apps']);
  });

  it('tells a user without apps to ask for access', async () => {
    signIn('user');
    const { v } = await started({ 'GET apps': [{ body: { apps: [] } }] });
    expect(v()).toMatchObject({
      empty: true,
      emptyTitle: 'No apps yet',
      emptyText: 'You don’t have access to any app yet. Ask your administrator for access.',
      emptyAction: null,
    });
  });

  it('leads an administrator without any app to adding one', async () => {
    signIn('admin');
    const { p, v } = await started({
      'GET apps': [{ body: { apps: [] } }],
      'GET admin/apps': [{ body: { apps: [] } }],
    });
    await vi.waitFor(() => expect(p.total).toBe(0));
    expect(v()).toMatchObject({ empty: true, emptyTitle: 'Add your first app', pendingHint: null });
    v().emptyAction?.onClick();
    expect(ctx.openApps).toHaveBeenCalled();
  });

  it('tells an administrator without access to the apps there are', async () => {
    signIn('admin');
    const { p, v } = await started({
      'GET apps': [{ body: { apps: [] } }],
      'GET admin/apps': [{ body: { apps: [admin('mail', 'paired', 'Mail')] } }],
    });
    await vi.waitFor(() => expect(p.total).toBe(1));
    expect(v().emptyText).toBe('You don’t have access to any app yet. Give yourself access in the administration.');
    expect(v().emptyAction?.label).toBe('Manage apps');
  });

  it('points an administrator to apps waiting for pairing', async () => {
    signIn('admin');
    const { p, v } = await started({
      'GET apps': [{ body: { apps: [MAIL] } }],
      'GET admin/apps': [{ body: { apps: [admin('mail', 'paired', 'Mail'), admin('wiki', 'pending')] } }],
    });
    await vi.waitFor(() => expect(p.pending).toEqual(['wiki']));
    expect(v().pendingHint?.text).toBe('wiki is waiting for pairing.');
    v().pendingHint?.onClick();
    expect(ctx.openApps).toHaveBeenCalled();
    p.pending = ['wiki', 'chat'];
    expect(v().pendingHint?.text).toBe('2 apps are waiting for pairing.');
  });

  it('keeps the tiles when the administration cannot be asked', async () => {
    signIn('admin');
    const { p, v } = await started({
      'GET apps': [{ body: { apps: [MAIL] } }],
      'GET admin/apps': [{ status: 500, body: { error: 'internal' } }],
    });
    await new Promise((r) => setTimeout(r, 0));
    expect(p.total).toBeNull();
    expect(v()).toMatchObject({ pendingHint: null, empty: false });
    expect(v().tiles).toHaveLength(1);
  });

  it('shows a failed first load with "Try again", a later failure keeps the tiles', async () => {
    signIn('user');
    const { p } = await started({
      'GET apps': [
        { status: 500, body: { error: 'internal' } },
        { body: { apps: [MAIL] } },
        { status: 500, body: { error: 'internal' } },
      ],
    });
    expect(p.status).toBe('failed');
    expect(p.failure).toBe('That didn’t work. Please try again.');
    await p.retry();
    expect(p.status).toBe('ready');
    p.refresh();
    await new Promise((r) => setTimeout(r, 0));
    expect(p.status).toBe('ready');
    expect(p.apps).toEqual([MAIL]);
  });

  it('writes only the latest load and nothing after leaving', async () => {
    signIn('user');
    let release: (v: { body: unknown }) => void = () => {};
    mockApi({
      'GET apps': [() => new Promise((r) => (release = r)), { body: { apps: [NOTES] } }],
    });
    const p = new HomePage();
    p.start();
    const first = release;
    await p.load();
    first({ body: { apps: [MAIL] } });
    await new Promise((r) => setTimeout(r, 0));
    expect(p.apps).toEqual([NOTES]);
    p.dispose();
    p.refresh();
    expect(p.apps).toEqual([NOTES]);
  });
});
