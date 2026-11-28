/**
 * Error pages and the text of the error toast when an action fails.
 *
 * The error pages cover the router's own (404 for an unknown address, 500)
 * and the gateway's (docs/app-contract.md › Errors): 403 `no_app_access`
 * (its own text: no access to the app), 403 `origin_mismatch`,
 * 502 `app_unreachable`, 503 `app_not_ready`, 504 `app_timeout`, and
 * "offline" when the hub itself does not answer.
 *
 * Plus the environment of the app for bug reports (`ErrorEnv`): the rows of a
 * developer page, the text of "Copy details" and the debug bar.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/errors.ts`) with the
 * gateway's codes, without the values of Mail's error page and without the
 * time in the account's time zone (`timeZoneOf`, `timeText`, from Mail's
 * settings). The app passes the view (`ErrorEnv.path`) already without
 * secrets; Mail redacted tokens in its own addresses (`redactPath`).
 */
import { isApiError, NetworkError, retryAfter } from './api.ts';
import type { IconName } from './icons.ts';
import { texts } from './i18n.ts';
import { waitText } from './wait.ts';

const t = texts({
  en: {
    e403_kicker: 'Error 403',
    e403_text: 'You don’t have permission to view this page. Contact your administrator.',
    e403_title: 'Access denied',
    e404_kicker: 'Error 404',
    e404_text: 'The URL is wrong or the page has moved.',
    e404_title: 'Page not found',
    e500_kicker: 'Error 500',
    e500_text: 'An unexpected error occurred. Try again.',
    e500_title: 'Something went wrong',
    e502_kicker: 'Error 502',
    e502_text: 'The app isn’t responding right now. Try again in a moment.',
    e502_title: 'App not reachable',
    e503_kicker: 'Error 503',
    e503_text: 'This app is still being set up. Try again later or contact your administrator.',
    e503_title: 'App not ready yet',
    e504_kicker: 'Error 504',
    e504_text: 'The app didn’t answer in time. Try again.',
    e504_title: 'The app took too long',
    env_browser: 'Browser',
    env_language: 'Language',
    env_layout: 'Layout',
    env_screen: 'Screen',
    env_screen_value: ({ width, height }: { width: number; height: number }) => `${width} × ${height} px`,
    env_timezone: 'Time zone',
    env_version: 'Version',
    env_version_value: ({ version, build }: { version: string; build: string }) => `${version} · Build ${build}`,
    env_view: 'View',
    layout_desktop: 'Desktop',
    layout_narrow: 'Narrow',
    act: 'That didn’t work. Please try again.',
    app_text: 'You’re signed in, but you don’t have access to this app. Ask your administrator for access.',
    app_title: 'No access to this app',
    limit: ({ wait }: { wait: string }) => `Too many requests. Try again in ${wait}.`,
    offline: 'No connection. Please try again.',
    offline_kicker: 'Offline',
    offline_text: 'Check your internet connection and try again.',
    offline_title: 'No connection',
  },
  de: {
    e403_kicker: 'Fehler 403',
    e403_text: 'Ihnen fehlt die Berechtigung für diese Seite. Wenden Sie sich an Ihre Administration.',
    e403_title: 'Kein Zugriff',
    e404_kicker: 'Fehler 404',
    e404_text: 'Die Adresse ist falsch oder die Seite wurde verschoben.',
    e404_title: 'Seite nicht gefunden',
    e500_kicker: 'Fehler 500',
    e500_text: 'Ein unerwarteter Fehler ist aufgetreten. Versuchen Sie es erneut.',
    e500_title: 'Etwas ist schiefgelaufen',
    e502_kicker: 'Fehler 502',
    e502_text: 'Die App antwortet gerade nicht. Versuchen Sie es gleich noch einmal.',
    e502_title: 'App nicht erreichbar',
    e503_kicker: 'Fehler 503',
    e503_text:
      'Diese App wird noch eingerichtet. Versuchen Sie es später erneut oder wenden Sie sich an Ihre Administration.',
    e503_title: 'App noch nicht bereit',
    e504_kicker: 'Fehler 504',
    e504_text: 'Die App hat nicht rechtzeitig geantwortet. Versuchen Sie es erneut.',
    e504_title: 'Die App braucht zu lange',
    env_browser: 'Browser',
    env_language: 'Sprache',
    env_layout: 'Layout',
    env_screen: 'Bildschirm',
    env_screen_value: ({ width, height }) => `${width} × ${height} px`,
    env_timezone: 'Zeitzone',
    env_version: 'Version',
    env_version_value: ({ version, build }) => `${version} · Build ${build}`,
    env_view: 'Ansicht',
    layout_desktop: 'Desktop',
    layout_narrow: 'Schmal',
    act: 'Das hat nicht geklappt. Bitte versuchen Sie es erneut.',
    app_text:
      'Sie sind angemeldet, haben aber keinen Zugriff auf diese App. Bitten Sie Ihre Administration um Zugriff.',
    app_title: 'Kein Zugriff auf diese App',
    limit: ({ wait }) => `Zu viele Anfragen. Versuchen Sie es in ${wait} erneut.`,
    offline: 'Keine Verbindung. Bitte versuchen Sie es erneut.',
    offline_kicker: 'Offline',
    offline_text: 'Prüfen Sie Ihre Internetverbindung und versuchen Sie es erneut.',
    offline_title: 'Keine Verbindung',
  },
});

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
    kicker: t.e404_kicker,
    title: t.e404_title,
    text: t.e404_text,
    retry: false,
  },
  '403': {
    icon: 'ti-lock',
    kicker: t.e403_kicker,
    title: t.e403_title,
    text: t.e403_text,
    retry: false,
  },
  /** 403 `no_app_access` of the gateway: signed in, but without access to the app. */
  app: {
    icon: 'ti-lock',
    kicker: t.e403_kicker,
    title: t.app_title,
    text: t.app_text,
    retry: false,
  },
  '500': { icon: 'ti-bug', kicker: t.e500_kicker, title: t.e500_title, text: t.e500_text, retry: true },
  '502': {
    icon: 'ti-plug-connected-x',
    kicker: t.e502_kicker,
    title: t.e502_title,
    text: t.e502_text,
    retry: true,
  },
  '503': {
    icon: 'ti-hourglass',
    kicker: t.e503_kicker,
    title: t.e503_title,
    text: t.e503_text,
    retry: true,
  },
  '504': {
    icon: 'ti-clock-exclamation',
    kicker: t.e504_kicker,
    title: t.e504_title,
    text: t.e504_text,
    retry: true,
  },
  offline: {
    icon: 'ti-wifi-off',
    kicker: t.offline_kicker,
    title: t.offline_title,
    text: t.offline_text,
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
export function failText(error: unknown, text: string = t.act()): string {
  return isOffline(error) ? t.offline() : text;
}

/** Text of a "Couldn’t load …" card: the wait time after 429, "No connection" without a response, otherwise "That didn’t work". */
export function loadFailText(error: unknown): string {
  return isApiError(error, 429) ? t.limit({ wait: waitText(retryAfter(error)) }) : failText(error);
}

/** Environment for "Copy details", the rows of a developer page and the debug bar. */
export interface ErrorEnv {
  /** Version of the app, e.g. `0.5.0`. */
  version: string;
  /** Build of the app, e.g. the short commit. */
  build: string;
  /** Narrow layout (`ui.isMobile`). */
  mobile: boolean;
  width: number;
  height: number;
  /** View: the page's path, without secrets (the app leaves out tokens in its addresses). */
  path: string;
  browser: string;
  timeZone: string;
  /** Language of the interface, e.g. `en`. */
  language: string;
  /** Region of the formats, e.g. `US`. */
  region: string;
}

/** Layout name: narrow (mobile) or desktop. */
export function layoutOf(env: Pick<ErrorEnv, 'mobile'>): string {
  return env.mobile ? t.layout_narrow() : t.layout_desktop();
}

/** Rows of the environment (a developer page and "Copy details"). */
export function envRows(env: ErrorEnv): [label: string, value: string][] {
  return [
    [t.env_version(), t.env_version_value({ version: env.version, build: env.build })],
    [t.env_layout(), layoutOf(env)],
    [t.env_screen(), t.env_screen_value({ width: Math.round(env.width), height: Math.round(env.height) })],
    [t.env_view(), env.path],
    [t.env_browser(), env.browser],
    [t.env_timezone(), env.timeZone],
    [t.env_language(), `${env.language.toUpperCase()} · ${env.region}`],
  ];
}

/** Heading, then per row of the environment "label: value". */
export function envText(title: string, env: ErrorEnv): string {
  return [title, ...envRows(env).map(([k, v]) => `${k}: ${v}`)].join('\n');
}

/** Text of "Copy details" on an error page: kicker and environment. */
export function errorDetails(code: ErrorCode, env: ErrorEnv): string {
  return envText(ERRS[code].kicker(), env);
}
