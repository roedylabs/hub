import { afterEach, describe, expect, it, vi } from 'vitest';
import { NoAccountPage, slugOf } from '#lib/no-account.svelte.ts';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { accountOf } from '#lib/test/stalwart.ts';

const MAIL = {
  slug: 'mail',
  name: 'Mail',
  description: '',
  url: '/mail/',
  icon: '/api/v1/apps/mail/icon.svg',
  status: 'online',
};

afterEach(() => vi.unstubAllGlobals());

describe('No account yet', () => {
  it('knows an app’s path from the hub’s', () => {
    expect(slugOf('/mail/inbox?x=1')).toBe('mail');
    expect(slugOf('/calendar')).toBe('calendar');
    expect(slugOf('/account/accounts')).toBeNull();
    expect(slugOf('/')).toBeNull();
  });

  it('names the app and leads to "Add account" with the same `next`', async () => {
    mockApi({ 'GET stalwart/accounts': [{ body: { accounts: [] } }], 'GET apps': [{ body: { apps: [MAIL] } }] });
    const p = new NoAccountPage('/mail/inbox', { leave: vi.fn() });
    await p.load();
    expect(p.ready).toBe(true);
    expect(p.app).toBe('Mail');
    expect(p.addHref).toBe('/account/accounts/new?next=%2Fmail%2Finbox');
    expect(new NoAccountPage('/', { leave: vi.fn() }).addHref).toBe('/account/accounts/new');
  });

  it('a user with accounts goes straight on', async () => {
    mockApi({ 'GET stalwart/accounts': [{ body: { accounts: [accountOf()] } }], 'GET apps': [{ body: { apps: [] } }] });
    const leave = vi.fn();
    const p = new NoAccountPage('/mail/', { leave });
    await p.load();
    expect(leave).toHaveBeenCalledWith('/mail/');
    expect(p.ready).toBe(false);
  });

  it('shows the page even if the requests fail', async () => {
    mockApi({ 'GET stalwart/accounts': [{ status: 500, body: {} }], 'GET apps': [{ status: 500, body: {} }] });
    const p = new NoAccountPage('/mail/', { leave: vi.fn() });
    await p.load();
    expect(p.ready).toBe(true);
    expect(p.app).toBeNull();
  });
});
