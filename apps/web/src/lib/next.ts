/**
 * `?next=` of the sign-in (docs/app-contract.md › Not signed in): where the
 * sign-in leads afterwards. Only a path on the same origin counts – it starts
 * with exactly one `/`, contains no `\` and no control characters; anything
 * else leads to the home page `/`, so that a link from outside can never send
 * the browser to another site after the sign-in (open redirect).
 *
 * `?reauth=1` (› Fresh sign-in for sensitive actions) asks for password and
 * second factor again while the session is still valid.
 */

/** Longest `next` that is followed; longer ones lead home. */
export const MAX_NEXT = 2048;

/** C0 and C1 control characters, DEL included. */
// eslint-disable-next-line no-control-regex -- control characters are exactly what is rejected
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;

/**
 * The checked path of `value` (path, query and fragment), otherwise `null`. `origin` is the hub's own; the parsed
 * address must stay on it.
 */
export function safeNext(value: string | null | undefined, origin: string = location.origin): string | null {
  if (!value || value.length > MAX_NEXT) return null;
  if (!value.startsWith('/') || value.startsWith('//')) return null;
  if (value.includes('\\') || CONTROL.test(value)) return null;
  let url: URL;
  try {
    url = new URL(value, origin);
  } catch {
    return null;
  }
  if (url.origin !== origin) return null;
  // Dot segments can leave two leading slashes after parsing (`/a/..//host`): another host for the browser.
  if (url.pathname.startsWith('//')) return null;
  const path = url.pathname + url.search + url.hash;
  // Back to the sign-in would only show it again.
  if (url.pathname === '/login' || url.pathname.startsWith('/login/')) return null;
  return path;
}

/** The parts of an address the sign-in reads (`URL`, SvelteKit's `page.url`). */
export interface Address {
  origin: string;
  searchParams: { get(name: string): string | null };
}

/** Target after the sign-in: `next` of the address if it is safe, otherwise the home page. */
export function nextOf(url: Address): string {
  return safeNext(url.searchParams.get('next'), url.origin) ?? '/';
}

/** A fresh sign-in was asked for (`reauth=1`). */
export function isReauth(url: Pick<Address, 'searchParams'>): boolean {
  return url.searchParams.get('reauth') === '1';
}

/** The sign-in with `next` (and `reauth`), for pages that need a signed-in user. */
export function loginHref(next: string, reauth = false): string {
  const params = new URLSearchParams();
  if (reauth) params.set('reauth', '1');
  if (next !== '/') params.set('next', next);
  const query = params.toString();
  return query ? `/login?${query}` : '/login';
}

/** First path segments that belong to the hub's web app; every other path belongs to an app behind the gateway. */
const HUB_SEGMENTS = new Set(['', 'login', 'setup', 'invite', 'account', 'admin']);

/** The hub's own page (navigated inside the web app) or an app's (a full page load through the gateway). */
export function isHubPath(path: string): boolean {
  const segment = path.split(/[/?#]/)[1] ?? '';
  return HUB_SEGMENTS.has(segment);
}
