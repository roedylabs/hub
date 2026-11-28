import { afterEach, describe, expect, it, vi } from 'vitest';
import { ErrorView, ui } from '@roedy/ui';
import { session } from '#lib/state/session.svelte.ts';
import { page } from '#lib/test/page.svelte.ts';
import { renderSignedIn, resetSignedIn, setPhone, signIn } from '#lib/test/signed-in.ts';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

afterEach(resetSignedIn);

describe('signed-in test helpers', () => {
  it('signs the test account in as administrator or user', () => {
    expect(signIn()).toMatchObject({ email: 'alex@example.com', role: 'admin' });
    expect(session).toMatchObject({ status: 'ready', authed: true, isAdmin: true, appName: 'Example Org' });
    signIn('user', { name: 'Other Org' });
    expect(session).toMatchObject({ isAdmin: false, appName: 'Other Org' });
  });

  it('renders a component signed in, with the address and the API answers', async () => {
    const { getByRole, calls, me } = renderSignedIn(ErrorView, {
      role: 'user',
      path: '/admin/users',
      props: { code: '403' },
      api: { 'GET admin/users': [{ status: 403, body: { error: 'forbidden' } }] },
    });
    expect(me.role).toBe('user');
    expect(page.url.pathname).toBe('/admin/users');
    expect(getByRole('heading', { level: 1, name: 'Access denied' })).toBeInTheDocument();
    await expect(fetch('/api/v1/admin/users')).resolves.toHaveProperty('status', 403);
    expect(calls.map((c) => c.path)).toEqual(['admin/users']);
  });

  it('turns the window into a phone and back', () => {
    setPhone(true);
    expect(ui.isMobile).toBe(true);
    setPhone(false);
    expect(ui.isMobile).toBe(false);
  });

  it('resets session, address and window', () => {
    signIn();
    page.url = new URL('http://localhost/account');
    setPhone(true);
    resetSignedIn();
    expect(session.user).toBeNull();
    expect(page.url.pathname).toBe('/');
    expect(ui.isMobile).toBe(false);
  });
});
