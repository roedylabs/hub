/**
 * Error pages and the text of the error toast when an action fails.
 *
 * The error pages cover the router's own (404 for an unknown address, 500)
 * and the gateway's (docs/app-contract.md › Errors): 403 `no_app_access`
 * (its own text: no access to the app), 403 `origin_mismatch`,
 * 502 `app_unreachable`, 503 `app_not_ready`, 504 `app_timeout`, and
 * "offline" when the hub itself does not answer.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/errors.ts`) with the
 * gateway's codes and without the developer details.
 */
import { isApiError, NetworkError, retryAfter } from '#lib/api.ts';
import type { IconName } from '#lib/icons.ts';
import { m } from '#lib/paraglide/messages.js';
import { waitText } from '#lib/wait.ts';

interface ErrorPage {
  icon: IconName;
  kicker: () => string;
  title: () => string;
  text: () => string;
  /** Offer "Try again" (reloads the page). */
  retry: boolean;
}

/** Error pages; texts as functions so that they follow the language. */
export const ERRS = {
  '404': {
    icon: 'ti-map-question',
    kicker: m.error_404_kicker,
    title: m.error_404_title,
    text: m.error_404_text,
    retry: false,
  },
  '403': {
    icon: 'ti-lock',
    kicker: m.error_403_kicker,
    title: m.error_403_title,
    text: m.error_403_text,
    retry: false,
  },
  /** 403 `no_app_access` of the gateway: signed in, but without access to the app. */
  app: {
    icon: 'ti-lock',
    kicker: m.error_403_kicker,
    title: m.error_app_title,
    text: m.error_app_text,
    retry: false,
  },
  '500': { icon: 'ti-bug', kicker: m.error_500_kicker, title: m.error_500_title, text: m.error_500_text, retry: true },
  '502': {
    icon: 'ti-plug-connected-x',
    kicker: m.error_502_kicker,
    title: m.error_502_title,
    text: m.error_502_text,
    retry: true,
  },
  '503': {
    icon: 'ti-hourglass',
    kicker: m.error_503_kicker,
    title: m.error_503_title,
    text: m.error_503_text,
    retry: true,
  },
  '504': {
    icon: 'ti-clock-exclamation',
    kicker: m.error_504_kicker,
    title: m.error_504_title,
    text: m.error_504_text,
    retry: true,
  },
  offline: {
    icon: 'ti-wifi-off',
    kicker: m.error_offline_kicker,
    title: m.error_offline_title,
    text: m.error_offline_text,
    retry: true,
  },
} as const satisfies Record<string, ErrorPage>;

export type ErrorCode = keyof typeof ERRS;

/** The gateway's error codes (docs/app-contract.md › Errors) and their pages. */
const GATEWAY: Readonly<Record<string, ErrorCode>> = {
  no_app_access: 'app',
  origin_mismatch: '403',
  not_found: '404',
  app_unreachable: '502',
  app_not_ready: '503',
  app_timeout: '504',
};

/** Error page for an HTTP status; unknown ones count as 500. */
export function errorCodeOf(status: number): ErrorCode {
  const code = String(status);
  return isErrorCode(code) && code !== 'offline' ? code : '500';
}

export function isErrorCode(value: string): value is ErrorCode {
  return Object.hasOwn(ERRS, value);
}

/** Error page for a gateway error code (`{"error": "<code>"}`), otherwise `null`. */
export function gatewayErrorOf(code: string | null | undefined): ErrorCode | null {
  return code && Object.hasOwn(GATEWAY, code) ? (GATEWAY[code] ?? null) : null;
}

/**
 * The gateway serves the hub's page with `<meta name="roedy-error" content="<code>">` when it answers a page load
 * under an app's path with an error itself; the page then shows that error at the same address.
 */
export function gatewayErrorIn(
  doc: Pick<Document, 'querySelector'> | undefined = globalThis.document,
): ErrorCode | null {
  const content = doc?.querySelector('meta[name="roedy-error"]')?.getAttribute('content');
  return gatewayErrorOf(content);
}

/** Without a response from the server (offline, network): `api()` then throws `NetworkError`. */
export function isOffline(error: unknown): boolean {
  return error instanceof NetworkError;
}

/**
 * Text of the error toast: without a connection "No connection.", otherwise the text of the caller – "That didn’t
 * work. …" if it has none of its own.
 */
export function failText(error: unknown, text: string = m.error_act()): string {
  return isOffline(error) ? m.error_offline() : text;
}

/** Text of a "Couldn’t load …" card: the wait time after 429, "No connection" without a response, otherwise "That didn’t work". */
export function loadFailText(error: unknown): string {
  return isApiError(error, 429) ? m.error_limit({ wait: waitText(retryAfter(error)) }) : failText(error);
}
