/**
 * The user's Stalwart accounts as every Roedy app sees them (contract § 6.1,
 * `GET /api/v1/stalwart/accounts`): the types, the account colour from
 * `MA_COLORS` of `@roedy/tokens` with its name, the letter of the account's
 * tile, the storage as text and the state as a short text. The hub's
 * Account › Accounts and the apps show accounts the same way with these and
 * `AccountTile`.
 */
import { MA_COLORS } from '@roedy/tokens';
import { texts } from './i18n.ts';

/** `state` of an account: `ok`, a new sign-in is needed, or blocked (custom server, policy off). */
export type StalwartAccountState = 'ok' | 'needs_sign_in' | 'blocked';

/** Storage as of the hub's last check; `used` and `total` `null` without a limit. */
export interface StalwartQuota {
  used: number | null;
  total: number | null;
  checked_at: string;
}

/** An account of `GET /api/v1/stalwart/accounts` (contract § 6.1). */
export interface StalwartAccount {
  id: string;
  /** The account's address, lower case. */
  email: string;
  /** `username` of the JMAP session. */
  username: string;
  /** Origin of the Stalwart server. */
  url: string;
  /** The registry server with this origin; `null` for a custom server. */
  server: { id: string; label: string } | null;
  /** Signed in at the server, or an access token the user entered. */
  auth: 'oauth' | 'token';
  session_url: string;
  capabilities: string[];
  default: boolean;
  /** Index into `MA_COLORS`. */
  color: number;
  state: StalwartAccountState;
  quota: StalwartQuota | null;
  created_at: string;
}

const t = texts({
  en: {
    apricot: 'Apricot',
    sage: 'Sage',
    lilac: 'Lilac',
    sky: 'Sky',
    rose: 'Rose',
    honey: 'Honey',
    ok: 'Connected',
    needsSignIn: 'Sign-in needed',
    blocked: 'Blocked',
    unlimited: ({ used }: { used: string }) => `${used} used`,
    usage: ({ used, total }: { used: string; total: string }) => `${used} of ${total}`,
    bytes: ({ n }: { n: string }) => `${n} B`,
    kb: ({ n }: { n: string }) => `${n} KB`,
    mb: ({ n }: { n: string }) => `${n} MB`,
    gb: ({ n }: { n: string }) => `${n} GB`,
    tb: ({ n }: { n: string }) => `${n} TB`,
  },
  de: {
    apricot: 'Aprikose',
    sage: 'Salbei',
    lilac: 'Flieder',
    sky: 'Himmel',
    rose: 'Rosé',
    honey: 'Honig',
    ok: 'Verbunden',
    needsSignIn: 'Anmeldung nötig',
    blocked: 'Gesperrt',
    unlimited: ({ used }) => `${used} belegt`,
    usage: ({ used, total }) => `${used} von ${total}`,
    bytes: ({ n }) => `${n} B`,
    kb: ({ n }) => `${n} KB`,
    mb: ({ n }) => `${n} MB`,
    gb: ({ n }) => `${n} GB`,
    tb: ({ n }) => `${n} TB`,
  },
});

/** Number of account colours. */
export const ACCOUNT_COLORS = MA_COLORS.length;

/** Fill and ink of an account colour; an index outside `MA_COLORS` wraps around. */
export function accountColor(index: number): { bg: string; ink: string } {
  const n = MA_COLORS.length;
  const [, bg, ink] = MA_COLORS[((Math.trunc(index) % n) + n) % n] ?? MA_COLORS[0];
  return { bg, ink };
}

/** Name of an account colour in the app's language ("Sage"), e.g. for a swatch. */
export function accountColorName(index: number): string {
  const n = MA_COLORS.length;
  const [name] = MA_COLORS[((Math.trunc(index) % n) + n) % n] ?? MA_COLORS[0];
  return t[name]();
}

/** Letter of the account's tile: the first letter or digit of the address, upper case. */
export function accountLetter(email: string): string {
  const first = [...email].find((c) => /[\p{L}\p{N}]/u.test(c));
  return (first ?? '?').toLocaleUpperCase();
}

/** Short text of the state ("Connected", "Sign-in needed", "Blocked"). */
export function accountStateText(state: StalwartAccountState): string {
  return state === 'needs_sign_in' ? t.needsSignIn() : state === 'blocked' ? t.blocked() : t.ok();
}

/** Bytes as text with one decimal from KB on (1024-based, like Stalwart). */
export function byteText(bytes: number, locale?: string): string {
  const units = [t.bytes, t.kb, t.mb, t.gb, t.tb] as const;
  let value = Math.max(0, bytes);
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  const n = new Intl.NumberFormat(locale, { maximumFractionDigits: unit === 0 ? 0 : 1 }).format(value);
  return units[unit]({ n });
}

/** Storage as text ("1.2 GB of 5 GB", without a limit "1.2 GB used"); `null` before the first check. */
export function quotaText(quota: StalwartQuota | null, locale?: string): string | null {
  if (!quota || quota.used === null) return null;
  const used = byteText(quota.used, locale);
  return quota.total === null ? t.unlimited({ used }) : t.usage({ used, total: byteText(quota.total, locale) });
}

/** Share of the storage in use (0–1); `null` without a limit or before the first check. */
export function quotaShare(quota: StalwartQuota | null): number | null {
  if (!quota || quota.used === null || !quota.total) return null;
  return Math.min(1, Math.max(0, quota.used / quota.total));
}
