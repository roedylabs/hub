/**
 * Address of the JMAP server of the system mails (Administration › Email):
 * the check of the backend's `normalize_url` (`api::system_mail`) in the
 * field, the stored form (origin) and the host for the texts. A plain module
 * (no runes): the URL objects here are values, not state.
 *
 * Adapted from Roedy Mail (`apps/web/src/lib/url.ts`), where the servers come
 * from a registry.
 */

/** Control characters like Rust `char::is_control` (category Cc). */
const CONTROL = /\p{Cc}/u;
/** Whitespace like Rust `char::is_whitespace`. */
const SPACE = /\p{White_Space}/u;
/** Labels of a DNS name like the backend (`a–z`, `0–9`, `-`, not at the edges). */
const DNS_LABEL = /^(?!-)[a-z0-9-]{1,63}(?<!-)$/;

/**
 * Address of the JMAP server like the backend's `normalize_url`: `https://`, for development also `http://` on
 * `localhost` or `127.0.0.1`, host as DNS name or IPv4 address, without credentials, path, query and fragment.
 * Returns the origin or the error.
 */
export function mailUrlOf(value: string): { origin: string } | { error: 'required' | 'invalid' } {
  const raw = value.trim();
  if (!raw) return { error: 'required' };
  let url: URL;
  try {
    url = new URL(raw.replace(/\/+$/, ''));
  } catch {
    return { error: 'invalid' };
  }
  const host = url.hostname;
  const ipv4 = /^\d{1,3}(\.\d{1,3}){3}$/.test(host);
  if (!host || host.startsWith('[') || (!ipv4 && !host.split('.').every((label) => DNS_LABEL.test(label))))
    return { error: 'invalid' };
  const loopback = host === 'localhost' || host === '127.0.0.1';
  const scheme = url.protocol === 'https:' || (url.protocol === 'http:' && loopback);
  // The URL parser removes tabs and line breaks silently; the backend rejects them as part of the host.
  const bare = !url.username && !url.password && url.pathname === '/' && !/[?#]/.test(raw);
  if (!scheme || !bare || SPACE.test(raw) || CONTROL.test(raw)) return { error: 'invalid' };
  return { origin: url.origin };
}

/** The server's address as the backend stores it (origin), otherwise the trimmed input. */
export function normUrl(value: string): string {
  const parsed = mailUrlOf(value);
  return 'origin' in parsed ? parsed.origin : value.trim();
}

/** Host with port of a valid address, otherwise empty. */
export function hostOf(value: string): string {
  const parsed = mailUrlOf(value);
  return 'origin' in parsed ? new URL(parsed.origin).host : '';
}
