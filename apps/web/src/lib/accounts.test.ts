import { ui } from '@roedy/ui';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AccountsPage, customOrigin, serverForEmail, targetOf, type AccountsNav } from '#lib/accounts.svelte.ts';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { lastAttempt, signInNav } from '#lib/stalwart-sign-in.ts';
import { accountOf, SERVER, serversOf, TOKEN_SERVER } from '#lib/test/stalwart.ts';

function nav(): AccountsNav & Record<string, ReturnType<typeof vi.fn>> {
  return { add: vi.fn(), list: vi.fn(), account: vi.fn(), servers: vi.fn(), leave: vi.fn() };
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  sessionStorage.clear();
  ui.reset();
});

async function page(adding: boolean, servers = serversOf(), next: string | null = null) {
  const n = nav();
  const api = mockApi({
    'GET stalwart/accounts': [{ body: { accounts: [accountOf()] } }],
    'GET stalwart/servers': [{ body: servers }],
  });
  const p = new AccountsPage(n, adding, next);
  await p.load();
  return { p, n, api };
}

describe('Account › Accounts', () => {
  it('finds the server by the address’s domain, or the only one', () => {
    expect(serverForEmail('a@example.org', [SERVER, TOKEN_SERVER])).toBe(TOKEN_SERVER);
    expect(serverForEmail('a@other.test', [SERVER, TOKEN_SERVER])).toBeNull();
    expect(serverForEmail('a@other.test', [SERVER])).toBe(SERVER);
    expect(customOrigin(' https://Mail.Other.test/x ')).toBe('https://mail.other.test');
    expect(customOrigin('http://mail.other.test')).toBe('');
    const form = { srv: 'other', email: '', url: 'https://mail.example.com' };
    // An address of its own with a registry server's origin is that server.
    expect(targetOf(form, [SERVER], false)).toEqual({ url: SERVER.url, server: SERVER });
    expect(targetOf({ ...form, url: 'https://own.test' }, [SERVER], false)).toBeNull();
    expect(targetOf({ ...form, url: 'https://own.test' }, [SERVER], true)).toEqual({
      url: 'https://own.test',
      server: null,
    });
    expect(targetOf({ srv: TOKEN_SERVER.id, email: '', url: '' }, [SERVER, TOKEN_SERVER], false)?.server).toBe(
      TOKEN_SERVER,
    );
  });

  it('loads accounts and servers; the list has no form, "Add account" leads to it', async () => {
    const { p, n } = await page(false);
    expect(p.status).toBe('ready');
    expect(p.accounts).toHaveLength(1);
    expect(p.form).toBeNull();
    p.add();
    expect(n.add).toHaveBeenCalled();
  });

  it('checks the form before sending anything', async () => {
    const { p, api } = await page(true);
    await p.submit();
    expect(p.form?.error?.field).toBe('email');
    p.update({ email: 'sam@other.test' });
    expect(p.form?.error).toBeNull();
    await p.submit();
    expect(p.form?.error).toMatchObject({ field: 'server', text: expect.stringContaining('other.test') as string });
    p.update({ srv: 'other', url: 'http://own.test' });
    await p.submit();
    expect(p.form?.error?.field).toBe('url');
    expect(api.calls.filter((c) => c.method === 'POST')).toEqual([]);
  });

  it('signs in at a server with the hub’s client: starts the sign-in and opens its page', async () => {
    const { p } = await page(true, serversOf(), '/mail/');
    const open = vi.spyOn(signInNav, 'open').mockImplementation(() => {});
    mockApi({ 'POST stalwart/sign-in': [{ body: { authorize_url: 'https://mail.example.com/authorize?x=1' } }] });
    p.update({ email: 'Sam@Example.com' });
    expect(p.canSignIn).toBe(true);
    expect(p.method).toBe('signin');
    await p.submit();
    expect(open).toHaveBeenCalledWith('https://mail.example.com/authorize?x=1');
    expect(p.form?.busy).toBe('open');
    expect(lastAttempt()).toEqual({
      server_id: SERVER.id,
      email: 'sam@example.com',
      return_to: '/mail/',
      host: 'mail.example.com',
    });
  });

  it('connects an access token on a server without the hub’s client, then opens the account', async () => {
    const { p, n } = await page(true);
    p.update({ email: 'sam@example.org' });
    expect(p.canSignIn).toBe(false);
    expect(p.method).toBe('token');
    await p.submit();
    expect(p.form?.error?.field).toBe('token');
    const added = accountOf({ id: '01JNEW', email: 'sam@example.org', auth: 'token', default: false });
    const api = mockApi({ 'POST stalwart/accounts': [{ status: 201, body: added }] });
    p.update({ token: ' api_key ' });
    await p.submit();
    expect(api.calls[0]?.body).toEqual({ server_id: TOKEN_SERVER.id, email: 'sam@example.org', token: 'api_key' });
    expect(p.accounts.map((a) => a.id)).toEqual(['01JACCT1', '01JNEW']);
    expect(p.form).toBeNull();
    expect(n.account).toHaveBeenCalledWith('01JNEW');
    expect(ui.toast?.text).toBe('sam@example.org connected');
  });

  it('a custom server takes an address and a token; with `next` it leads back there', async () => {
    const { p, n } = await page(true, serversOf([SERVER], true), '/mail/');
    p.update({ email: 'sam@own.test', srv: 'other', url: 'https://own.test/' });
    const api = mockApi({ 'POST stalwart/accounts': [{ status: 201, body: accountOf({ id: '01JC', server: null }) }] });
    p.update({ token: 'tok' });
    await p.submit();
    expect(api.calls[0]?.body).toEqual({ url: 'https://own.test', email: 'sam@own.test', token: 'tok' });
    expect(n.leave).toHaveBeenCalledWith('/mail/');
  });

  it('shows the API’s errors in the form, locks after 429', async () => {
    const { p } = await page(true);
    p.update({ email: 'sam@example.org', token: 'tok' });
    mockApi({
      'POST stalwart/accounts': [
        { status: 409, body: { error: 'dup', account: { id: '01JACCT1', email: 'sam@example.org' } } },
        { status: 429, body: { error: 'rate_limited', retry_after: 30 } },
      ],
    });
    await p.submit();
    expect(p.form?.error).toMatchObject({ act: 'open', account: '01JACCT1' });
    expect(p.form?.busy).toBeNull();
    p.update({ token: 'tok2' });
    await p.submit();
    expect(p.lockLeft).toBeGreaterThan(25);
    // Locked: nothing goes out.
    await p.submit();
    p.dispose();
  });

  it('"Cancel" goes back to the list, or to `next`', async () => {
    const a = await page(true);
    a.p.cancel();
    expect(a.n.list).toHaveBeenCalled();
    const b = await page(true, serversOf(), '/mail/');
    b.p.cancel();
    expect(b.n.leave).toHaveBeenCalledWith('/mail/');
  });

  it('a failed first load shows the card', async () => {
    mockApi({ 'GET stalwart/accounts': [{ status: 500, body: {} }], 'GET stalwart/servers': [{ body: serversOf() }] });
    const p = new AccountsPage(nav(), false);
    await p.load();
    expect(p.status).toBe('failed');
    expect(p.failure).not.toBe('');
  });
});
