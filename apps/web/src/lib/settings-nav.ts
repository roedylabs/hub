/**
 * Navigation of the account and administration area: the pages in their
 * groups, which page and area an address belongs to, and the values of the
 * navigation component (`SettingsNav.svelte`). Pages take their heading and
 * subtitle from here (`pageOf(path)`).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/settings-nav.ts`), trimmed to
 * the hub's pages: Stalwart accounts and servers in place of mail accounts and
 * servers, no app settings, no "Advanced settings" and no filter.
 */
import type { IconName, LogoValues } from '@roedy/ui';
import { m } from '#lib/paraglide/messages.js';

/** Area of the navigation: the account's own settings, or the administration. */
export type PanelSection = 'account' | 'admin';

/** Pages of the navigation. */
export type PanelRoute =
  | '/account/profile'
  | '/account/security'
  | '/account/accounts'
  | '/account/appearance'
  | '/account/language'
  | '/admin/users'
  | '/admin/apps'
  | '/admin/organization'
  | '/admin/branding'
  | '/admin/stalwart'
  | '/admin/email'
  | '/admin/audit-log';

/** Page of the navigation. */
export interface PanelPage {
  /** Stable key of the page (`profile`, `users` …). */
  id: string;
  route: PanelRoute;
  label: string;
  icon: IconName;
  /** Subtitle of the page, below its heading. */
  desc: string;
}

export interface PanelGroup {
  /** Heading of the group; `null` without one. */
  label: string | null;
  pages: PanelPage[];
}

const page = (id: string, route: PanelRoute, label: string, icon: IconName, desc: string): PanelPage => ({
  id,
  route,
  label,
  icon,
  desc,
});

/** Settings: the account first, then how the hub looks and speaks. */
function accountGroups(): PanelGroup[] {
  return [
    {
      label: m.nav_group_account(),
      pages: [
        page('profile', '/account/profile', m.nav_profile(), 'ti-user', m.nav_profile_desc()),
        page('security', '/account/security', m.nav_security(), 'ti-shield', m.nav_security_desc()),
        page('accounts', '/account/accounts', m.nav_accounts(), 'ti-at', m.nav_accounts_desc()),
      ],
    },
    {
      label: m.nav_group_display(),
      pages: [
        page('appearance', '/account/appearance', m.nav_appearance(), 'ti-palette', m.nav_appearance_desc()),
        page('language', '/account/language', m.nav_language(), 'ti-language', m.nav_language_desc()),
      ],
    },
  ];
}

/** Administration: people, apps and the organization first, then "System". */
function adminGroups(): PanelGroup[] {
  return [
    {
      label: null,
      pages: [
        page('users', '/admin/users', m.nav_users(), 'ti-users', m.nav_users_desc()),
        page('apps', '/admin/apps', m.nav_apps(), 'ti-apps', m.nav_apps_desc()),
        page('organization', '/admin/organization', m.nav_organisation(), 'ti-building', m.nav_organisation_desc()),
        page('branding', '/admin/branding', m.nav_branding(), 'ti-brush', m.nav_branding_desc()),
      ],
    },
    {
      label: m.nav_group_system(),
      pages: [
        page('stalwart', '/admin/stalwart', m.nav_stalwart(), 'ti-server-2', m.nav_stalwart_desc()),
        page('email', '/admin/email', m.nav_email(), 'ti-mail', m.nav_email_desc()),
        page('audit-log', '/admin/audit-log', m.nav_audit(), 'ti-list-details', m.nav_audit_desc()),
      ],
    },
  ];
}

/** Groups of an area. */
export function panelGroups(section: PanelSection): PanelGroup[] {
  return section === 'admin' ? adminGroups() : accountGroups();
}

/** Start of each area (`/account`, `/admin`): its list on mobile, on the desktop the first page. */
export const SECTION_HOME: Readonly<Record<PanelSection, '/account' | '/admin'>> = {
  account: '/account',
  admin: '/admin',
};

/** First page of each area, where its start leads on the desktop. */
export const FIRST_PAGE: Readonly<Record<PanelSection, PanelRoute>> = {
  account: '/account/profile',
  admin: '/admin/users',
};

function within(path: string, prefix: string): boolean {
  return path === prefix || path.startsWith(`${prefix}/`);
}

/** "No account yet" (`/account/no-account`): a page of its own without the shell, like the home page. */
export const NO_ACCOUNT_PATH = '/account/no-account';

/**
 * Area of an address with the shell: `/admin` and below, `/account` and below except the guest page
 * "Confirm email" (`/account/confirm-email/…`) and "No account yet"; otherwise `null`.
 */
export function sectionOf(path: string): PanelSection | null {
  if (within(path, '/admin')) return 'admin';
  if (within(path, '/account') && !within(path, '/account/confirm-email') && !within(path, NO_ACCOUNT_PATH))
    return 'account';
  return null;
}

/** The start of an area itself (`/account`, `/admin`, also with a trailing slash). */
export function isSectionHome(path: string): boolean {
  const p = path.length > 1 ? path.replace(/\/$/, '') : path;
  return p === SECTION_HOME.account || p === SECTION_HOME.admin;
}

/** Name of an area: heading of its list on mobile and of its navigation. */
export function sectionName(section: PanelSection): string {
  return section === 'admin' ? m.shell_admin() : m.shell_settings();
}

/** Page for the address, also for subpages (`/admin/users/…`); `null` outside the pages. */
export function pageOf(path: string): PanelPage | null {
  const section = sectionOf(path);
  if (!section) return null;
  return (
    panelGroups(section)
      .flatMap((g) => g.pages)
      .find((p) => within(path, p.route)) ?? null
  );
}

/** Entry of the navigation. */
export interface NavEntry {
  id: string;
  label: string;
  icon: IconName;
  /** Current page: card, tile in the accent colour. */
  on: boolean;
  fw: number;
  onClick: () => void;
}

export interface NavGroupValues {
  label: string;
  hasLabel: boolean;
  items: NavEntry[];
}

/** Props of the navigation component. */
export interface SettingsNavValues {
  isMobile: boolean;
  appName: string;
  logo: LogoValues;
  backLabel: string;
  /** "Back to home" of the desktop bar. */
  panelBack: () => void;
  /** Name of the area: label of the navigation, heading of the list on mobile. */
  mPanelH: string;
  settingsGroups: NavGroupValues[];
}

export interface NavInput {
  path: string;
  open: (page: PanelPage) => void;
}

/** Groups with the entries of the address's area; outside the areas none. */
export function navGroupValues(input: NavInput): NavGroupValues[] {
  const section = sectionOf(input.path);
  if (!section) return [];
  const current = pageOf(input.path);
  return panelGroups(section).map((g) => ({
    label: g.label ?? '',
    hasLabel: !!g.label,
    items: g.pages.map((p): NavEntry => {
      const on = current?.route === p.route;
      return { id: p.id, label: p.label, icon: p.icon, on, fw: on ? 800 : 600, onClick: () => input.open(p) };
    }),
  }));
}
