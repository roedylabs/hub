import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AppPage, AppsPage, internalUrlError, slugError, slugErrorText } from '#lib/admin-apps.svelte.ts';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { session } from '#lib/state/session.svelte.ts';
import { ui } from '@roedy/ui';
import { offlineApp, pairedApp, PAIRING, pendingApp } from '#lib/test/apps.ts';
import { resetSignedIn, signIn } from '#lib/test/signed-in.ts';

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const APPS = [pairedApp(), pendingApp(), offlineApp()];

beforeEach(() => {
  signIn('admin');
});

afterEach(() => {
  resetSignedIn();
});

async function list(routes: Parameters<typeof mockApi>[0] = {}) {
  const api = mockApi({ 'GET admin/apps': Array.from({ length: 5 }, () => ({ body: { apps: APPS } })), ...routes });
  const p = new AppsPage();
  p.start();
  await vi.waitFor(() => expect(p.status).toBe('ready'));
  return { p, ...api };
}

describe('slug and internal address', () => {
  it('checks the slug like the server', () => {
    expect(slugError('')).toBe('required');
    expect(slugError('mail')).toBeNull();
    expect(slugError('my-app2')).toBeNull();
    expect(slugError('m')).toBe('invalid');
    expect(slugError('2mail')).toBe('invalid');
    expect(slugError('mail-')).toBe('invalid');
    expect(slugError('Mail')).toBe('invalid');
    expect(slugError('a'.repeat(33))).toBe('invalid');
    expect(slugError('admin')).toBe('reserved');
    expect(slugError('settings')).toBe('reserved');
    expect(slugErrorText('taken')).toBe('Another app already uses this address.');
  });

  it('accepts only scheme, host and port as the internal address', () => {
    expect(internalUrlError('')).toBe('required');
    expect(internalUrlError('http://roedy-mail:8080')).toBeNull();
    expect(internalUrlError('https://10.0.0.5/')).toBeNull();
    expect(internalUrlError('roedy-mail:8080')).toBe('invalid');
    expect(internalUrlError('ftp://roedy-mail')).toBe('invalid');
    expect(internalUrlError('http://roedy-mail:8080/mail')).toBe('invalid');
    expect(internalUrlError('http://user:pw@roedy-mail')).toBe('invalid');
    expect(internalUrlError('http://roedy-mail/?')).toBe('invalid');
    expect(internalUrlError('http://roedy-mail#x')).toBe('invalid');
  });
});

describe('Administration › Apps list', () => {
  it('loads the apps; a failed first load shows the card, a later one a toast', async () => {
    mockApi({
      'GET admin/apps': [
        { status: 500, body: { error: 'internal' } },
        { body: { apps: APPS } },
        { status: 500, body: { error: 'internal' } },
      ],
    });
    const p = new AppsPage();
    p.start();
    await vi.waitFor(() => expect(p.status).toBe('failed'));
    expect(p.failure).toBe('That didn’t work. Please try again.');
    await p.retry();
    expect(p.rows.map((a) => a.slug)).toEqual(['mail', 'wiki', 'notes']);
    await p.load();
    expect(ui.toast).toMatchObject({ error: true, text: 'The list couldn’t be updated.' });
    expect(p.rows).toHaveLength(3);
  });

  it('moves an app and sends the whole order, announcing the new position', async () => {
    const { p, calls } = await list({ 'PATCH admin/apps': [{ body: { apps: APPS } }, { body: { apps: APPS } }] });
    p.move('notes', -1);
    expect(p.rows.map((a) => a.slug)).toEqual(['mail', 'notes', 'wiki']);
    expect(p.moved).toBe('Notes moved to position 2 of 3');
    p.move('notes', -1);
    // Beyond the ends nothing happens.
    p.move('notes', -1);
    p.move('wiki', 1);
    await vi.waitFor(() => expect(calls.filter((c) => c.method === 'PATCH')).toHaveLength(2));
    expect(calls.filter((c) => c.method === 'PATCH').map((c) => c.body)).toEqual([
      { order: ['mail', 'notes', 'wiki'] },
      { order: ['notes', 'mail', 'wiki'] },
    ]);
    expect(calls.map((c) => c.method)).not.toContain('POST');
  });

  it('reloads the list when the apps changed meanwhile (422) or the order failed', async () => {
    const { p, calls } = await list({
      'PATCH admin/apps': [
        { status: 422, body: { error: 'invalid', fields: { order: { code: 'invalid' } } } },
        { status: 500, body: { error: 'internal' } },
      ],
    });
    p.move('wiki', -1);
    await vi.waitFor(() => expect(ui.toast?.text).toBe('The apps changed in the meantime. The list was updated.'));
    await vi.waitFor(() => expect(p.rows.map((a) => a.slug)).toEqual(['mail', 'wiki', 'notes']));
    p.move('wiki', -1);
    await vi.waitFor(() => expect(ui.toast?.text).toBe('The order wasn’t saved. Please try again.'));
    await vi.waitFor(() => expect(calls.filter((c) => c.path === 'admin/apps' && c.method === 'GET')).toHaveLength(3));
  });

  it('reconciles a failed order only after the queued writes settled', async () => {
    let fail: (a: { status: number; body: unknown }) => void = () => {};
    let pass: (a: { body: unknown }) => void = () => {};
    const final = [APPS[2], APPS[0], APPS[1]].filter((a) => a !== undefined);
    const { p, calls } = await list({
      'PATCH admin/apps': [() => new Promise((r) => (fail = r)), () => new Promise((r) => (pass = r))],
      'GET admin/apps': [{ body: { apps: APPS } }, { body: { apps: final } }],
    });
    p.move('notes', -1);
    p.move('notes', -1);
    fail({ status: 500, body: { error: 'internal' } });
    await tick();
    // The second write is still queued: no reload yet.
    expect(calls.filter((c) => c.method === 'GET')).toHaveLength(1);
    await vi.waitFor(() => expect(calls.filter((c) => c.method === 'PATCH')).toHaveLength(2));
    pass({ body: { apps: final } });
    await vi.waitFor(() => expect(calls.filter((c) => c.method === 'GET')).toHaveLength(2));
    await vi.waitFor(() => expect(p.rows.map((a) => a.slug)).toEqual(['notes', 'mail', 'wiki']));
  });

  it('checks the form before sending and adds the app with its pairing code', async () => {
    const created = pendingApp('2026-10-26T12:15:00Z', { slug: 'chat' });
    const { p, calls } = await list({ 'POST admin/apps': [{ status: 201, body: { app: created, pairing: PAIRING } }] });
    p.openAdd();
    await p.submitAdd();
    expect(p.add).toMatchObject({ slugErr: 'required', urlErr: 'required' });
    p.setSlug('Chat');
    expect(p.add).toMatchObject({ slug: 'chat', slugErr: null, urlErr: 'required' });
    p.setInternal(' http://chat.internal:4000 ');
    await p.submitAdd();
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({
      slug: 'chat',
      internal_url: 'http://chat.internal:4000',
    });
    expect(p.add).toBeNull();
    expect(p.shown).toEqual({ slug: 'chat', name: 'chat', pairing: PAIRING });
    expect(p.rows.map((a) => a.slug)).toContain('chat');
    expect(ui.toast?.text).toBe('App /chat/ added');
    p.closeShown();
    expect(p.shown).toBeNull();
  });

  it('shows the server’s field errors at the fields', async () => {
    const { p } = await list({
      'POST admin/apps': [
        { status: 422, body: { error: 'invalid', fields: { slug: { code: 'taken' } } } },
        { status: 500, body: { error: 'internal' } },
      ],
    });
    p.openAdd();
    p.setSlug('mail2');
    p.setInternal('http://mail2.internal');
    await p.submitAdd();
    expect(p.add).toMatchObject({ busy: false, slugErr: 'taken', urlErr: null });
    p.setSlug('mail3');
    await p.submitAdd();
    expect(p.add).toMatchObject({ busy: false, slugErr: null });
    expect(ui.toast).toMatchObject({ error: true, text: 'That didn’t work. Please try again.' });
  });

  it('copies the pairing code', async () => {
    const writeText = vi.fn(() => Promise.resolve());
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } });
    const { p } = await list();
    p.shown = { slug: 'wiki', name: 'wiki', pairing: PAIRING };
    p.copyShown();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Pairing code copied'));
    expect(writeText).toHaveBeenCalledWith(PAIRING.code);
  });

  it('reloads the account when the role was lost', async () => {
    const refresh = vi.spyOn(session, 'refreshUser').mockResolvedValue();
    mockApi({ 'GET admin/apps': [{ status: 403, body: { error: 'forbidden' } }] });
    const p = new AppsPage();
    p.start();
    await vi.waitFor(() => expect(p.status).toBe('failed'));
    expect(refresh).toHaveBeenCalled();
  });
});

describe('Administration › Apps › an app', () => {
  const leave = vi.fn();

  afterEach(() => leave.mockClear());

  async function app(routes: Parameters<typeof mockApi>[0], slug = 'mail') {
    const api = mockApi(routes);
    const p = new AppPage(slug, { leave });
    p.start();
    await vi.waitFor(() => expect(p.status).not.toBe('loading'));
    await tick();
    return { p, ...api };
  }

  const USERS = {
    users: [
      { id: 'u1', status: 'active', name: 'Alex Example', email: 'alex@example.com', role: 'admin', app_role: 'admin' },
      { id: 'u2', status: 'locked', name: null, email: 'sam@example.com', role: 'user', app_role: null },
    ],
    invites: [{ id: 'i1', status: 'invited', name: null, email: 'kim@example.com', role: 'user', app_role: 'user' }],
  };

  it('loads a paired app with who has access; a pending one without', async () => {
    const { p, calls } = await app({
      'GET admin/apps/mail': [{ body: { app: pairedApp() } }],
      'GET admin/apps/mail/users': [{ body: USERS }],
    });
    expect(p.app?.slug).toBe('mail');
    expect(p.users.map((u) => u.id)).toEqual(['u1', 'u2', 'i1']);
    expect(p.usersStatus).toBe('ready');
    const pending = await app({ 'GET admin/apps/wiki': [{ body: { app: pendingApp() } }] }, 'wiki');
    expect(pending.p.status).toBe('ready');
    expect(pending.calls.map((c) => c.path)).toEqual(['admin/apps/wiki']);
    expect(calls.map((c) => c.path)).toEqual(['admin/apps/mail', 'admin/apps/mail/users']);
  });

  it('says "not found" for an unknown app and keeps a failed users list retryable', async () => {
    const gone = await app({ 'GET admin/apps/nope': [{ status: 404, body: { error: 'not_found' } }] }, 'nope');
    expect(gone.p.status).toBe('gone');
    const { p } = await app({
      'GET admin/apps/mail': [{ body: { app: pairedApp() } }],
      'GET admin/apps/mail/users': [{ status: 500, body: { error: 'internal' } }, { body: USERS }],
    });
    expect(p.usersStatus).toBe('failed');
    await p.loadUsers();
    expect(p.usersStatus).toBe('ready');
  });

  it('makes a new pairing code and shows it once', async () => {
    const { p, calls } = await app(
      {
        'GET admin/apps/wiki': [{ body: { app: pendingApp() } }],
        'POST admin/apps/wiki/pairing-code': [{ body: { app: pendingApp('2026-10-26T12:15:00Z'), pairing: PAIRING } }],
      },
      'wiki',
    );
    await p.newCode();
    expect(calls.at(-1)).toMatchObject({ method: 'POST', path: 'admin/apps/wiki/pairing-code' });
    expect(p.pairing).toEqual(PAIRING);
    expect(p.app?.pairing?.expires_at).toBe('2026-10-26T12:15:00Z');
    p.closeCode();
    expect(p.pairing).toBeNull();
  });

  it('saves the access default at once and goes back on an error', async () => {
    const { p, calls } = await app({
      // After the failure the page loads the app again: the server still has "selected".
      'GET admin/apps/mail': [
        { body: { app: pairedApp() } },
        { body: { app: pairedApp({ access_default: 'selected' }) } },
      ],
      'GET admin/apps/mail/users': [{ body: USERS }, { body: USERS }],
      'PATCH admin/apps/mail': [
        { body: { app: pairedApp({ access_default: 'selected' }) } },
        { status: 500, body: { error: 'internal' } },
      ],
    });
    p.setDefault('selected');
    expect(p.app?.access_default).toBe('selected');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Default for new users saved'));
    expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ access_default: 'selected' });
    p.setDefault('all');
    expect(p.app?.access_default).toBe('all');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('The default wasn’t saved. Please try again.'));
    await vi.waitFor(() => expect(calls.filter((c) => c.path === 'admin/apps/mail')).toHaveLength(4));
    expect(p.app?.access_default).toBe('selected');
  });

  it('shows the latest default even when an earlier save of it fails', async () => {
    let first: (a: { status: number; body: unknown }) => void = () => {};
    const { p } = await app({
      'GET admin/apps/mail': [
        { body: { app: pairedApp() } },
        { body: { app: pairedApp({ access_default: 'selected' }) } },
      ],
      'GET admin/apps/mail/users': [{ body: USERS }, { body: USERS }],
      'PATCH admin/apps/mail': [
        () => new Promise((r) => (first = r)),
        { body: { app: pairedApp({ access_default: 'all' }) } },
        { body: { app: pairedApp({ access_default: 'selected' }) } },
      ],
    });
    p.setDefault('selected');
    p.setDefault('all');
    p.setDefault('selected');
    first({ status: 500, body: { error: 'internal' } });
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Default for new users saved'));
    await tick();
    expect(p.app?.access_default).toBe('selected');
  });

  it('reloads after a failed default save only once the queued saves settled', async () => {
    let first: (a: { status: number; body: unknown }) => void = () => {};
    let last: (a: { body: unknown }) => void = () => {};
    const { p, calls } = await app({
      'GET admin/apps/mail': [
        { body: { app: pairedApp() } },
        { body: { app: pairedApp({ access_default: 'selected' }) } },
      ],
      'GET admin/apps/mail/users': [{ body: USERS }, { body: USERS }],
      'PATCH admin/apps/mail': [
        () => new Promise((r) => (first = r)),
        { body: { app: pairedApp({ access_default: 'all' }) } },
        () => new Promise((r) => (last = r)),
      ],
    });
    const gets = () => calls.filter((c) => c.method === 'GET' && c.path === 'admin/apps/mail').length;
    p.setDefault('selected');
    p.setDefault('all');
    p.setDefault('selected');
    first({ status: 500, body: { error: 'internal' } });
    await vi.waitFor(() => expect(calls.filter((c) => c.method === 'PATCH')).toHaveLength(3));
    // The last save is still on its way: no reload yet.
    expect(gets()).toBe(1);
    last({ body: { app: pairedApp({ access_default: 'selected' }) } });
    await vi.waitFor(() => expect(gets()).toBe(2));
    await tick();
    expect(p.app?.access_default).toBe('selected');
  });

  it('waits for a pending default save before reloading after a failed new code', async () => {
    let save: (a: { body: unknown }) => void = () => {};
    const { p, calls } = await app({
      'GET admin/apps/mail': [
        { body: { app: pairedApp() } },
        { body: { app: pairedApp({ access_default: 'selected' }) } },
      ],
      'GET admin/apps/mail/users': [{ body: USERS }, { body: USERS }],
      'PATCH admin/apps/mail': [() => new Promise((r) => (save = r))],
      'POST admin/apps/mail/pairing-code': [{ status: 500, body: { error: 'internal' } }],
    });
    const gets = () => calls.filter((c) => c.method === 'GET' && c.path === 'admin/apps/mail').length;
    p.setDefault('selected');
    await p.newCode();
    expect(ui.toast).toMatchObject({ error: true });
    // The save is still on its way: a reload now could read the old default and overwrite the saved one.
    expect(gets()).toBe(1);
    save({ body: { app: pairedApp({ access_default: 'selected' }) } });
    await vi.waitFor(() => expect(gets()).toBe(2));
    await tick();
    expect(p.app?.access_default).toBe('selected');
  });

  it('keeps a default chosen while a new pairing code was on its way', async () => {
    let code: (a: { body: unknown }) => void = () => {};
    const { p } = await app({
      'GET admin/apps/mail': [{ body: { app: pairedApp() } }],
      'GET admin/apps/mail/users': [{ body: USERS }],
      'POST admin/apps/mail/pairing-code': [() => new Promise((r) => (code = r))],
      'PATCH admin/apps/mail': [{ body: { app: pairedApp({ access_default: 'selected' }) } }],
    });
    const asked = p.newCode();
    p.setDefault('selected');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Default for new users saved'));
    // The code's response was made before the save: it still names the old default.
    code({ body: { app: pairedApp({ pairing: { expires_at: PAIRING.expires_at } }), pairing: PAIRING } });
    await asked;
    expect(p.pairing).toEqual(PAIRING);
    expect(p.app?.pairing?.expires_at).toBe(PAIRING.expires_at);
    expect(p.app?.access_default).toBe('selected');
  });

  it('keeps a default saved before a slower new pairing code answers', async () => {
    let code: (a: { body: unknown }) => void = () => {};
    const { p } = await app({
      'GET admin/apps/mail': [{ body: { app: pairedApp() } }],
      'GET admin/apps/mail/users': [{ body: USERS }],
      'PATCH admin/apps/mail': [{ body: { app: pairedApp({ access_default: 'selected' }) } }],
      'POST admin/apps/mail/pairing-code': [() => new Promise((r) => (code = r))],
    });
    p.setDefault('selected');
    const asked = p.newCode();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Default for new users saved'));
    code({ body: { app: pairedApp(), pairing: PAIRING } });
    await asked;
    expect(p.app?.access_default).toBe('selected');
  });

  it('drops a load that started before a default change', async () => {
    let late: (a: { body: unknown }) => void = () => {};
    const { p } = await app({
      'GET admin/apps/mail': [{ body: { app: pairedApp() } }, () => new Promise((r) => (late = r))],
      'GET admin/apps/mail/users': [{ body: USERS }, { body: USERS }],
      'PATCH admin/apps/mail': [{ body: { app: pairedApp({ access_default: 'selected' }) } }],
    });
    void p.load();
    p.setDefault('selected');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Default for new users saved'));
    late({ body: { app: pairedApp() } });
    await tick();
    expect(p.app?.access_default).toBe('selected');
  });

  it('removes the app after confirmation and leaves the page', async () => {
    const { p, calls } = await app({
      'GET admin/apps/mail': [{ body: { app: pairedApp() } }],
      'GET admin/apps/mail/users': [{ body: USERS }],
      'DELETE admin/apps/mail': [{ status: 204 }],
    });
    p.remove();
    expect(ui.confirm).toMatchObject({ title: 'Remove Mail?', ok: 'Remove app', danger: true });
    ui.confirmOk();
    await vi.waitFor(() => expect(leave).toHaveBeenCalled());
    expect(calls.at(-1)).toMatchObject({ method: 'DELETE', path: 'admin/apps/mail' });
    expect(ui.toast?.text).toBe('Mail removed');
  });

  it('treats an app removed meanwhile as removed, and a failure as not', async () => {
    const gone = await app({
      'GET admin/apps/mail': [{ body: { app: pairedApp() } }],
      'GET admin/apps/mail/users': [{ body: USERS }],
      'DELETE admin/apps/mail': [{ status: 404, body: { error: 'not_found' } }],
    });
    gone.p.remove();
    ui.confirmOk();
    await vi.waitFor(() => expect(leave).toHaveBeenCalledTimes(1));
    const { p } = await app({
      'GET admin/apps/mail': [{ body: { app: pairedApp() } }, { body: { app: pairedApp() } }],
      'GET admin/apps/mail/users': [{ body: USERS }, { body: USERS }],
      'DELETE admin/apps/mail': [{ status: 500, body: { error: 'internal' } }],
    });
    p.remove();
    ui.confirmOk();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('The app wasn’t removed. Please try again.'));
    expect(leave).toHaveBeenCalledTimes(1);
  });

  it('shows "not found" when the app goes while acting on it', async () => {
    const { p } = await app(
      {
        'GET admin/apps/wiki': [{ body: { app: pendingApp() } }],
        'POST admin/apps/wiki/pairing-code': [{ status: 404, body: { error: 'not_found' } }],
      },
      'wiki',
    );
    await p.newCode();
    expect(p.status).toBe('gone');
    expect(ui.toast?.text).toBe('This app no longer exists.');
  });
});
