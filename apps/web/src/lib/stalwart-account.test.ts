import { ui } from '@roedy/ui';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { AccountDetailPage } from '#lib/stalwart-account.svelte.ts';
import { lastAttempt, signInNav } from '#lib/stalwart-sign-in.ts';
import { accountOf, SERVER, serversOf, TOKEN_SERVER } from '#lib/test/stalwart.ts';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  sessionStorage.clear();
  ui.reset();
});

async function page(account = accountOf(), servers = serversOf()) {
  const nav = { list: vi.fn(), servers: vi.fn() };
  mockApi({
    'GET stalwart/accounts': [{ body: { accounts: [account] } }],
    'GET stalwart/servers': [{ body: servers }],
  });
  const p = new AccountDetailPage(account.id, nav);
  await p.load();
  return { p, nav };
}

/** An answer that comes when the test says so. */
function later<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

describe('One Stalwart account', () => {
  it('loads the account with its server; an unknown one is missing', async () => {
    const { p } = await page();
    expect(p.status).toBe('ready');
    expect(p.host).toBe('mail.example.com');
    expect(p.server).toEqual(SERVER);
    expect(p.again).toBe('signin');
    expect(p.capabilities).toEqual(['mail', 'contacts']);
    mockApi({ 'GET stalwart/accounts': [{ body: { accounts: [] } }], 'GET stalwart/servers': [{ body: serversOf() }] });
    const other = new AccountDetailPage('01JX', { list: vi.fn(), servers: vi.fn() });
    await other.load();
    expect(other.status).toBe('missing');
  });

  it('knows how the account signs in again', async () => {
    expect((await page(accountOf({ auth: 'token' }))).p.again).toBe('token');
    expect((await page(accountOf({ state: 'blocked', server: null }))).p.again).toBe('blocked');
    const lost = accountOf({ server: { id: TOKEN_SERVER.id, label: TOKEN_SERVER.label } });
    expect((await page(lost)).p.again).toBe('noclient');
  });

  it('saves the colour and the default account, and puts them back on failure', async () => {
    const { p } = await page(accountOf({ default: false }));
    const api = mockApi({
      'PATCH stalwart/accounts/01JACCT1': [
        { body: accountOf({ color: 3, default: false }) },
        { status: 500, body: {} },
        { body: accountOf({ color: 3, default: true }) },
      ],
    });
    await p.setColor(3);
    expect(api.calls[0]?.body).toEqual({ color: 3 });
    expect(p.account?.color).toBe(3);
    await p.setColor(4);
    expect(p.account?.color).toBe(3);
    expect(ui.toast?.error).toBe(true);
    await p.makeDefault();
    expect(api.calls[2]?.body).toEqual({ default: true });
    expect(p.account?.default).toBe(true);
    expect(ui.toast?.text).toBe('alex@example.com is now the default account');
  });

  it('"Check" stores the result; a failure shows with the account’s state', async () => {
    const { p } = await page();
    const checked = accountOf({ quota: { used: 1, total: 2, checked_at: '2026-11-28T10:00:00.000Z' } });
    mockApi({
      'POST stalwart/accounts/01JACCT1/check': [
        { body: checked },
        { status: 409, body: { error: 'needs_sign_in' } },
        { status: 502, body: { error: 'check_failed', reason: 'unreachable' } },
      ],
    });
    await p.check();
    expect(p.checked).toBe('ok');
    expect(p.account?.quota?.used).toBe(1);
    expect(p.turns).toBe(2);
    await p.check();
    expect(p.account?.state).toBe('needs_sign_in');
    expect(p.checked).toMatchObject({ title: 'Sign-in needed' });
    await p.check();
    expect(p.checked).toMatchObject({ title: 'Server unreachable' });
  });

  it('"Sign in again" starts the sign-in for the account and returns to its page', async () => {
    const { p } = await page();
    const open = vi.spyOn(signInNav, 'open').mockImplementation(() => {});
    const api = mockApi({
      'POST stalwart/sign-in': [
        { status: 502, body: { error: 'sign_in_unavailable', reason: 'unreachable' } },
        { body: { authorize_url: 'https://mail.example.com/authorize' } },
      ],
    });
    await p.startAgain();
    expect(p.signIn?.error?.title).toBe('Sign-in page not available');
    await p.startAgain();
    expect(api.calls[1]?.body).toEqual({ account_id: '01JACCT1', return_to: '/account/accounts/01JACCT1' });
    expect(open).toHaveBeenCalledWith('https://mail.example.com/authorize');
    expect(lastAttempt()?.account_id).toBe('01JACCT1');
  });

  it('an account with an access token takes a new one', async () => {
    const { p } = await page(accountOf({ auth: 'token', state: 'needs_sign_in' }));
    await p.startAgain();
    expect(p.reauth).not.toBeNull();
    await p.submitReauth();
    expect(p.reauth?.error?.field).toBe('token');
    const api = mockApi({
      'POST stalwart/accounts/01JACCT1/reauth': [
        { status: 422, body: { error: 'check_failed', reason: 'auth_failed' } },
        { body: accountOf({ auth: 'token' }) },
      ],
    });
    p.updateReauth({ token: 'bad' });
    await p.submitReauth();
    expect(p.reauth?.error?.title).toBe('Invalid access token');
    p.updateReauth({ token: ' good ' });
    await p.submitReauth();
    expect(api.calls[1]?.body).toEqual({ token: 'good' });
    expect(p.reauth).toBeNull();
    expect(p.account?.state).toBe('ok');
    expect(ui.toast?.text).toBe('Signed in to alex@example.com again');
  });

  it('"Remove" asks first, then removes and goes back to the list', async () => {
    const { p, nav } = await page();
    const api = mockApi({ 'DELETE stalwart/accounts/01JACCT1': [{ status: 204 }] });
    p.remove();
    expect(ui.confirm?.title).toBe('Remove alex@example.com?');
    expect(api.calls).toEqual([]);
    ui.confirmOk();
    await vi.waitFor(() => expect(nav.list).toHaveBeenCalled());
    expect(api.calls[0]?.method).toBe('DELETE');
    expect(ui.toast?.text).toBe('alex@example.com removed');
  });

  it('after overlapping failed colour saves it shows the colour the server holds', async () => {
    const { p } = await page(accountOf({ color: 0 }));
    const first = later<{ status: number; body: unknown }>();
    const second = later<{ status: number; body: unknown }>();
    mockApi({ 'PATCH stalwart/accounts/01JACCT1': [() => first.promise, () => second.promise] });
    const one = p.setColor(1);
    const two = p.setColor(2);
    expect(p.account?.color).toBe(2);
    first.resolve({ status: 500, body: {} });
    await one;
    expect(p.account?.color).toBe(2);
    second.resolve({ status: 500, body: {} });
    await two;
    expect(p.account?.color).toBe(0);
  });

  it('shows the colour a later-settling save confirmed once no save is pending', async () => {
    const { p } = await page(accountOf({ color: 0 }));
    const first = later<{ status: number; body: unknown }>();
    const second = later<{ status: number; body: unknown }>();
    mockApi({ 'PATCH stalwart/accounts/01JACCT1': [() => first.promise, () => second.promise] });
    const one = p.setColor(1);
    const two = p.setColor(2);
    second.resolve({ status: 500, body: {} });
    await two;
    expect(p.account?.color).toBe(0);
    first.resolve({ status: 200, body: accountOf({ color: 1 }) });
    await one;
    expect(p.account?.color).toBe(1);
  });

  it('a check that finds a new sign-in needed keeps a colour saved meanwhile', async () => {
    const { p } = await page(accountOf({ color: 0 }));
    const checked = later<{ status: number; body: unknown }>();
    mockApi({
      'POST stalwart/accounts/01JACCT1/check': [() => checked.promise],
      'PATCH stalwart/accounts/01JACCT1': [{ body: accountOf({ color: 1 }) }],
    });
    const check = p.check();
    await p.setColor(1);
    checked.resolve({ status: 409, body: { error: 'needs_sign_in' } });
    await check;
    expect(p.account?.state).toBe('needs_sign_in');
    expect(p.account?.color).toBe(1);
  });

  it('"Sign in again" waits for the countdown and does not leave once the page is gone', async () => {
    const { p } = await page();
    const open = vi.spyOn(signInNav, 'open').mockImplementation(() => {});
    const started = later<{ status?: number; body: unknown }>();
    const api = mockApi({
      'POST stalwart/sign-in': [
        { status: 429, body: { error: 'rate_limited', retry_after: 30 } },
        () => started.promise,
      ],
    });
    await p.startAgain();
    expect(p.clock.left('rate')).toBeGreaterThan(0);
    await p.startAgain();
    expect(api.calls).toHaveLength(1);
    p.clock.clear('rate');
    const again = p.startAgain();
    p.dispose();
    started.resolve({ body: { authorize_url: 'https://mail.example.com/authorize' } });
    await again;
    expect(open).not.toHaveBeenCalled();
    expect(lastAttempt()).toBeNull();
  });
});
