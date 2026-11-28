/**
 * Addresses of Stalwart servers as Administration › Stalwart servers and
 * Account › Accounts check them in the field. The backend's rules
 * (`stalwart::normalize_url`, `normalize_custom_url`) apply in addition; the
 * pages show its field errors with the same texts.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/url.ts`) without `normUrl`
 * (the hub's setup has no server).
 */

/** Host with port if given; empty for anything that is not an http(s) address with a host. */
export function hostOf(url: string): string {
  try {
    const parsed = new URL(url);
    return /^https?:$/.test(parsed.protocol) && parsed.hostname ? parsed.host : '';
  } catch {
    return '';
  }
}

/** Origin of an http(s) address; empty if it is none. */
export function originOf(url: string): string {
  try {
    const parsed = new URL(url);
    return /^https?:$/.test(parsed.protocol) && parsed.hostname ? parsed.origin : '';
  } catch {
    return '';
  }
}

/**
 * `http://` on `localhost` or `127.0.0.1`: allowed for registry servers, but unencrypted and only for development
 * on this machine.
 */
export function localHttp(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === 'http:' && /^(localhost|127\.0\.0\.1)$/i.test(parsed.hostname);
  } catch {
    return false;
  }
}

/** `http://` with another host: rejected with "Use an HTTPS URL. …". */
export function httpOther(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === 'http:' && parsed.hostname !== '' && !localHttp(url);
  } catch {
    return false;
  }
}

/**
 * Domain `name.tld`, lower case, ASCII. Like the backend: at most 253 characters, labels of 1 to 63 characters
 * without a hyphen at the start or end.
 */
export function isDomain(domain: string): boolean {
  return domain.length <= 253 && /^(?!-)[a-z0-9-]{1,63}(?<!-)(\.(?!-)[a-z0-9-]{1,63}(?<!-))+$/.test(domain);
}

/** At most this many domains per server (backend). */
export const MAX_DOMAINS = 100;

/** Domains from an input: separated by comma, semicolon or whitespace, lower case, without a leading `@`. */
export function domainsOf(value: string): string[] {
  return value
    .split(/[\s,;]+/)
    .map((d) => d.trim().toLowerCase().replace(/^@/, ''))
    .filter(Boolean);
}
