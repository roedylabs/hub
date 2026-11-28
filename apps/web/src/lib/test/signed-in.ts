/**
 * Pages of the account and administration area in tests: a signed-in account
 * (administrator or user) without loading the session, the page rendered with
 * the API answers it needs (`mockApi`), a phone-sized window, and the reset
 * for `afterEach`. Not a test file.
 *
 * A test of a page still mocks `$app/navigation` (and `$app/state` with
 * `#lib/test/page.svelte.ts` if the page reads it) itself: `vi.mock` only
 * applies in the test file.
 */
import { render } from '@testing-library/svelte';
import { vi } from 'vitest';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { session, type Me, type PublicConfig } from '#lib/state/session.svelte.ts';
import { BRAND0, ui } from '@roedy/ui';
import { page } from '#lib/test/page.svelte.ts';
import { sessionOf } from '#lib/test/session.ts';

/** Public configuration of the tests: the organization "Example Org" with support address and legal links. */
export const TEST_CONFIG: PublicConfig = {
  name: 'Example Org',
  branding: BRAND0,
  org: {
    name: 'Example Org',
    website: null,
    help: null,
    email: 'support@example.com',
    imprint: 'https://example.com/imprint',
    privacy: 'https://example.com/privacy',
    terms: null,
    source: null,
  },
  setup_required: false,
};

/** The signed-in account of the tests. */
export const TEST_USER = { id: '01JUSER', email: 'alex@example.com', name: 'Alex Example' } as const;

export type Role = Me['role'];

/** Signs the test account in with `role` (session ready, without requests); returns it. */
export function signIn(role: Role = 'admin', config: Partial<PublicConfig> = {}): Me {
  const info = sessionOf({ ...TEST_USER, role });
  session.config = { ...TEST_CONFIG, ...config };
  session.user = info.user;
  session.current = info.session;
  session.status = 'ready';
  session.failure = null;
  return info.user;
}

/** Routes of `mockApi`: answers per `METHOD path`. */
export type ApiRoutes = Parameters<typeof mockApi>[0];

/** A component as `render` takes it. */
type Renderable = Parameters<typeof render>[0];

export interface SignedInOptions {
  /** Role of the signed-in account; administrator by default. */
  role?: Role;
  /** Answers of the API for the page (`mockApi`), e.g. `{ 'GET admin/users': [{ body: [...] }] }`. */
  api?: ApiRoutes;
  /** Address of the page for the stand-in of `page` (`#lib/test/page.svelte.ts`). */
  path?: string;
  config?: Partial<PublicConfig>;
  /** Props of the component. */
  props?: Record<string, unknown>;
}

/**
 * Renders `component` (a page) for a signed-in account: session, address and API answers are set first. Returns
 * the view and the recorded API calls (`calls`).
 */
export function renderSignedIn(component: Renderable, options: SignedInOptions = {}) {
  ui.animations = false;
  const me = signIn(options.role, options.config);
  if (options.path) page.url = new URL(`http://localhost${options.path}`);
  const api = mockApi(options.api ?? {});
  const view = render(component, { props: options.props ?? {} });
  return { ...view, ...api, me };
}

/** Window width of a phone (below `MOBILE_BP`) or the jsdom default (desktop). */
const PHONE = 393;
let desktopWidth: number | null = null;

/** Turns the window into a phone (`ui.isMobile`) or back into the desktop. */
export function setPhone(on: boolean): void {
  if (on && desktopWidth === null) desktopWidth = window.innerWidth;
  const width = on ? PHONE : (desktopWidth ?? window.innerWidth);
  if (!on) desktopWidth = null;
  Object.defineProperty(window, 'innerWidth', { value: width, configurable: true });
  window.dispatchEvent(new Event('resize'));
}

/** Undoes the sign-in, the window size, the address and the stubs; for `afterEach`. */
export function resetSignedIn(): void {
  setPhone(false);
  vi.unstubAllGlobals();
  session.config = null;
  session.user = null;
  session.current = null;
  session.status = 'idle';
  session.failure = null;
  prefs.reset();
  ui.reset();
  page.url = new URL('http://localhost/');
  page.params = {};
  page.error = null;
  page.status = 200;
}
