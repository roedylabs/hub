import { describe, expect, it } from 'vitest';
import { isGuestPath, needsUser, pageTitle, redirectFor } from './routes.ts';

describe('routes', () => {
  it('knows the guest pages and those that need a user', () => {
    for (const path of [
      '/setup',
      '/login',
      '/login/forgot',
      '/login/reset/t',
      '/invite/t',
      '/account/confirm-email/t',
    ]) {
      expect(isGuestPath(path), path).toBe(true);
      expect(needsUser(path), path).toBe(false);
    }
    for (const path of ['/', '/account', '/account/security', '/admin', '/admin/users']) {
      expect(needsUser(path), path).toBe(true);
    }
    // Other paths belong to the apps (or are the router's 404).
    for (const path of ['/mail/', '/accounting', '/administrator', '/invite'])
      expect(needsUser(path), path).toBe(false);
  });

  it('redirects to the setup, the sign-in and on', () => {
    const guest = { setupRequired: false, authed: false };
    const user = { setupRequired: false, authed: true };
    expect(redirectFor('/login', { setupRequired: true, authed: false })).toEqual({ to: 'setup' });
    expect(redirectFor('/setup', { setupRequired: true, authed: false })).toBeNull();
    expect(redirectFor('/', guest)).toEqual({ to: 'login', next: '/' });
    expect(redirectFor('/account', { ...guest, here: '/account?x=1' })).toEqual({ to: 'login', next: '/account?x=1' });
    expect(redirectFor('/setup', guest)).toEqual({ to: 'login', next: '/' });
    expect(redirectFor('/login', guest)).toBeNull();
    expect(redirectFor('/mail/', guest)).toBeNull();
    expect(redirectFor('/login', user)).toEqual({ to: 'next' });
    expect(redirectFor('/login', { ...user, reauth: true })).toBeNull();
    expect(redirectFor('/login/forgot', user)).toBeNull();
    expect(redirectFor('/setup', user)).toEqual({ to: 'home' });
    expect(redirectFor('/setup', { ...user, leavingSetup: true })).toBeNull();
    expect(redirectFor('/', user)).toBeNull();
  });

  it('titles the pages', () => {
    expect(pageTitle({ path: '/', appName: 'Example Org' })).toBe('Example Org');
    expect(pageTitle({ path: '/login', appName: 'Example Org', auth: 'Sign in · Example Org' })).toBe(
      'Sign in · Example Org',
    );
    expect(pageTitle({ path: '/account/confirm-email/t', appName: 'Example Org' })).toBe(
      'Confirm new email address – Example Org',
    );
  });
});
