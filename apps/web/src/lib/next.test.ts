import { describe, expect, it } from 'vitest';
import { isHubPath, isReauth, loginHref, MAX_NEXT, nextOf, safeNext } from '#lib/next.ts';

const ORIGIN = 'https://hub.example.com';
const at = (query: string) => new URL(`${ORIGIN}/login${query}`);

describe('`next` of the sign-in (docs/app-contract.md › Not signed in)', () => {
  it('follows paths on the same origin with query and fragment', () => {
    expect(safeNext('/', ORIGIN)).toBe('/');
    expect(safeNext('/mail/mailbox/inbox', ORIGIN)).toBe('/mail/mailbox/inbox');
    expect(safeNext('/mail/?q=a%20b#top', ORIGIN)).toBe('/mail/?q=a%20b#top');
    expect(safeNext('/account/security', ORIGIN)).toBe('/account/security');
    // Encoded slashes stay part of the path.
    expect(safeNext('/%2F%2Fevil.example', ORIGIN)).toBe('/%2F%2Fevil.example');
    expect(safeNext('/a/../mail/', ORIGIN)).toBe('/mail/');
  });

  it.each([
    // Not a path
    'https://evil.example/',
    'http://hub.example.com/mail/',
    'javascript:alert(1)',
    'data:text/html,<script>alert(1)</script>',
    'mail/inbox',
    '?next=/x',
    '#top',
    ' /mail/',
    // Two leading slashes: another host (protocol-relative)
    '//evil.example',
    '//evil.example/path',
    '///evil.example',
    // … also after resolving dot segments
    '/a/..//evil.example',
    '/a/%2e%2e//evil.example',
    '/./a/../..//evil.example/path',
    // A backslash counts as a slash in browsers
    '/\\evil.example',
    '\\\\evil.example',
    '/mail\\..\\..\\evil',
    // Control characters (C0, DEL, C1), also tabs and line breaks the URL parser drops
    '/\t/evil.example',
    '/\n/evil.example',
    '/\r/evil.example',
    '/mail/\u0000',
    '/mail/\u007f',
    '/mail/\u0085',
    '/\u001b/evil.example',
  ])('refuses %j', (value) => {
    expect(safeNext(value, ORIGIN)).toBeNull();
  });

  it('refuses empty, missing and overlong values', () => {
    expect(safeNext('', ORIGIN)).toBeNull();
    expect(safeNext(null, ORIGIN)).toBeNull();
    expect(safeNext(undefined, ORIGIN)).toBeNull();
    expect(safeNext(`/${'a'.repeat(MAX_NEXT)}`, ORIGIN)).toBeNull();
    expect(safeNext(`/${'a'.repeat(MAX_NEXT - 1)}`, ORIGIN)).not.toBeNull();
  });

  it('does not lead back to the sign-in itself', () => {
    expect(safeNext('/login', ORIGIN)).toBeNull();
    expect(safeNext('/login?next=/mail/', ORIGIN)).toBeNull();
    expect(safeNext('/login/forgot', ORIGIN)).toBeNull();
    expect(safeNext('/loginx', ORIGIN)).toBe('/loginx');
  });

  it('goes home for a missing or refused `next`', () => {
    expect(nextOf(at(''))).toBe('/');
    expect(nextOf(at('?next=%2Fmail%2F'))).toBe('/mail/');
    expect(nextOf(at('?next=https%3A%2F%2Fevil.example'))).toBe('/');
    expect(nextOf(at('?next=%2F%2Fevil.example'))).toBe('/');
    expect(nextOf(at('?next=%2F%5Cevil.example'))).toBe('/');
    expect(nextOf(at('?next=%2F%0D%0Aevil'))).toBe('/');
    expect(nextOf(at('?next=%2Fa%2F..%2F%2Fevil.example'))).toBe('/');
  });

  it('knows a fresh sign-in only by `reauth=1`', () => {
    expect(isReauth(at('?reauth=1&next=%2Fmail%2F'))).toBe(true);
    expect(isReauth(at('?reauth=true'))).toBe(false);
    expect(isReauth(at(''))).toBe(false);
  });

  it('builds the sign-in address with `next` and `reauth`', () => {
    expect(loginHref('/')).toBe('/login');
    expect(loginHref('/account/security?tab=2')).toBe('/login?next=%2Faccount%2Fsecurity%3Ftab%3D2');
    expect(loginHref('/mail/', true)).toBe('/login?reauth=1&next=%2Fmail%2F');
    expect(nextOf(new URL(loginHref('/account/x#y'), ORIGIN))).toBe('/account/x#y');
  });

  it('tells the hub’s own pages from the apps’', () => {
    for (const path of ['/', '/login', '/account/security', '/admin', '/setup', '/invite/abc', '/?x=1']) {
      expect(isHubPath(path), path).toBe(true);
    }
    for (const path of ['/mail/', '/mail', '/contacts/x', '/accounting', '/login-help']) {
      expect(isHubPath(path), path).toBe(false);
    }
  });
});
