/**
 * Routes of the hub's web app: which pages need a sign-in, where the entry
 * redirects to (setup, sign-in, home page) and the document title per route
 * (WCAG 2.4.2).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/routes.ts` and
 * `apps/web/src/lib/page-title.ts`), trimmed to the hub's routes.
 */
import { type ErrorCode, ERRS } from '@roedy/ui';
import { m } from '#lib/paraglide/messages.js';
import { isSectionHome, pageOf, sectionName, sectionOf } from '#lib/settings-nav.ts';

export const HOME_PATH = '/';
export const LOGIN_PATH = '/login';
export const SETUP_PATH = '/setup';

/** Guest pages: setup, sign-in and the links from mails. They stay reachable when signed in. */
const GUEST_PREFIXES = ['/setup', '/login', '/invite/', '/account/confirm-email/'];

/** Pages that need a signed-in user (the sign-in comes first, with `next`). */
const USER_PREFIXES = ['/account', '/admin'];

function matches(path: string, prefix: string): boolean {
  return path === prefix || path.startsWith(prefix.endsWith('/') ? prefix : `${prefix}/`);
}

export function isGuestPath(path: string): boolean {
  return GUEST_PREFIXES.some((prefix) => matches(path, prefix) || (prefix.endsWith('/') && path.startsWith(prefix)));
}

/** A page of the hub that needs a signed-in user. */
export function needsUser(path: string): boolean {
  if (isGuestPath(path)) return false;
  return path === HOME_PATH || USER_PREFIXES.some((prefix) => matches(path, prefix));
}

export type Redirect = { to: 'setup' } | { to: 'login'; next: string } | { to: 'home' } | { to: 'next' };

/**
 * Redirect after loading the session, otherwise `null`: without an administrator only the setup
 * (`setup_required`); without sign-in the pages that need a user go to the sign-in (with `next`, the current
 * address); when signed in the sign-in leads on to `next` – except for a fresh sign-in (`reauth`) – and the setup
 * home. The setup that has just signed the administrator in (`leavingSetup`) stays until it has opened the home page
 * itself.
 */
export function redirectFor(
  path: string,
  state: {
    setupRequired: boolean;
    authed: boolean;
    reauth?: boolean;
    leavingSetup?: boolean;
    /** Path, query and fragment of the open page. */
    here?: string;
  },
): Redirect | null {
  if (state.setupRequired) return path === SETUP_PATH ? null : { to: 'setup' };
  if (!state.authed) {
    if (path === SETUP_PATH) return { to: 'login', next: HOME_PATH };
    return needsUser(path) ? { to: 'login', next: state.here ?? path } : null;
  }
  if (path === SETUP_PATH) return state.leavingSetup ? null : { to: 'home' };
  if (path === LOGIN_PATH) return state.reauth ? null : { to: 'next' };
  return null;
}

/** Separator between the parts of the title. */
const SEP = ' – ';

/** Heading of the page: of an account or administration page, of the start of its area, of "Confirm email". */
function headingOf(path: string): string {
  if (path.startsWith('/account/confirm-email/')) return m.ec_ask_title();
  const page = pageOf(path);
  if (page) return page.label;
  const section = sectionOf(path);
  return section && isSectionHome(path) ? sectionName(section) : '';
}

/**
 * Document title: the guest page's own (`auth`, set by its shell on `/setup`, `/login…` and `/invite/…`), otherwise
 * "‹Page› – ‹Hub name›" (e.g. "Profile – Roedy Hub", or the error page's title instead of the page: "Access
 * denied") or only the hub's name.
 */
export function pageTitle(input: {
  path: string;
  appName: string;
  auth?: string | null;
  /** Error page in place of the page (e.g. 403 on `/admin/…` for a user). */
  error?: ErrorCode | null;
}): string {
  if (input.error) return [ERRS[input.error].title(), input.appName].join(SEP);
  if (input.auth) return input.auth;
  const heading = headingOf(input.path);
  return heading ? [heading, input.appName].join(SEP) : input.appName;
}
