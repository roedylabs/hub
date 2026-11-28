/**
 * Apps as the API returns them: the home page's tiles (`GET apps`,
 * docs/app-contract.md › Browser API) and the administration's apps
 * (`GET admin/apps`, `api::apps` of the server) with their roles, and the
 * texts that derive from them – name and description in the interface's
 * language, the access choices per app ("No access" or a role) and the
 * app's status.
 */
import { ago, type IconName } from '@roedy/ui';
import { m } from '#lib/paraglide/messages.js';
import { getLocale } from '#lib/paraglide/runtime.js';

/** Tile of the home page (`GET apps`). */
export interface HomeApp {
  slug: string;
  name: string;
  description: string;
  /** `/<slug><home>`. */
  url: string;
  /** Versioned address of the icon, for an `<img>`. */
  icon: string;
  status: 'online' | 'offline';
}

/** Texts of a manifest by language (`en` and `de` always). */
export type Texts = Readonly<Record<string, string>>;

/** Role of an app's manifest. */
export interface AppRoleDef {
  id: string;
  name: Texts;
  description: Texts;
  default?: boolean;
  admin?: boolean;
}

export type AccessDefault = 'all' | 'selected';

/** Event delivery of an app (`events` of `admin/apps`). */
export interface AppEvents {
  pending: number;
  attempts: number;
  failing_since: string | null;
  retry_at: string | null;
  last_error: string | null;
  given_up: boolean;
}

/** App of the administration (`GET admin/apps`); manifest fields are `null` while it waits for pairing. */
export interface AdminApp {
  id: string;
  slug: string;
  internal_url: string;
  public_url: string;
  state: 'pending' | 'paired';
  manifest_id: string | null;
  version: string | null;
  name: Texts | null;
  description: Texts | null;
  home: string | null;
  url: string | null;
  icon: string | null;
  roles: AppRoleDef[] | null;
  access_default: AccessDefault;
  position: number;
  health: 'online' | 'offline' | null;
  health_at: string | null;
  online_at: string | null;
  contacted_at: string | null;
  paired_at: string | null;
  created_at: string;
  /** Open pairing code (its expiry); the code itself only comes with the response that made it. */
  pairing: { expires_at: string } | null;
  user_count: number;
  events: AppEvents;
}

/** A new pairing code (`POST admin/apps`, `POST admin/apps/:slug/pairing-code`): shown once. */
export interface Pairing {
  code: string;
  /** In groups of four. */
  display: string;
  expires_at: string;
}

/** Text in the interface's language, otherwise English. */
export function textIn(texts: Texts | null | undefined, fallback = ''): string {
  if (!texts) return fallback;
  return texts[getLocale()] ?? texts.en ?? fallback;
}

/** Shown name of an administration app: from the manifest, while pending its slug. */
export function appName(app: Pick<AdminApp, 'name' | 'slug'>): string {
  return textIn(app.name, app.slug);
}

export const isPaired = (app: Pick<AdminApp, 'state'>) => app.state === 'paired';

/** The manifest's default role (exactly one), otherwise the first. */
export function defaultRole(app: Pick<AdminApp, 'roles'>): AppRoleDef | null {
  const roles = app.roles ?? [];
  return roles.find((r) => r.default) ?? roles[0] ?? null;
}

/** Name of a role of the app; an unknown id stays as it is. */
export function roleName(app: Pick<AdminApp, 'roles'>, id: string): string {
  const role = app.roles?.find((r) => r.id === id);
  return role ? textIn(role.name, id) : id;
}

/** What a user or an invitation gets in an app: no access (`null`) or a role. */
export type AccessChoice = string | null;

/** Option of an access choice ("No access", the roles). */
export interface AccessOption {
  /** `null`: no access. */
  value: AccessChoice;
  label: string;
  /** The role's description, or for "No access" none. */
  desc: string;
  /** The manifest's default role. */
  isDefault: boolean;
  on: boolean;
}

/** "No access" and the app's roles in the manifest's order, `current` checked. */
export function accessOptions(app: Pick<AdminApp, 'roles'>, current: AccessChoice): AccessOption[] {
  const none: AccessOption = { value: null, label: m.access_none(), desc: '', isDefault: false, on: current === null };
  return [
    none,
    ...(app.roles ?? []).map((r) => ({
      value: r.id,
      label: textIn(r.name, r.id),
      desc: textIn(r.description),
      isDefault: !!r.default,
      on: current === r.id,
    })),
  ];
}

/** What a new user gets in the app by its default: the default role with "everyone", otherwise none. */
export function defaultAccess(app: Pick<AdminApp, 'roles' | 'access_default'>): AccessChoice {
  return app.access_default === 'all' ? (defaultRole(app)?.id ?? null) : null;
}

/** Status of an administration app for the list and its page. */
export interface AppStatus {
  /** `pending`, `online`, `offline` (failed check) or `unknown` (paired, not checked yet). */
  kind: 'pending' | 'online' | 'offline' | 'unknown';
  label: string;
  /** Details: since when offline, the pairing code's validity, the last contact. */
  detail: string;
  icon: IconName;
}

/** Status of an app at `now`. */
export function appStatus(app: AdminApp, now = Date.now()): AppStatus {
  if (app.state === 'pending') {
    const exp = app.pairing ? Date.parse(app.pairing.expires_at) : NaN;
    const open = Number.isFinite(exp) && exp > now;
    return {
      kind: 'pending',
      label: m.apps_status_pending(),
      detail: open ? m.apps_code_valid({ left: until(exp, now) }) : m.apps_code_expired(),
      icon: 'ti-plug',
    };
  }
  if (app.health === 'online') {
    return { kind: 'online', label: m.apps_status_online(), detail: contactOf(app, now), icon: 'ti-plug-connected' };
  }
  if (app.health === 'offline') {
    const since = app.online_at ? Date.parse(app.online_at) : NaN;
    return {
      kind: 'offline',
      label: m.apps_status_offline(),
      detail: Number.isFinite(since) ? m.apps_offline_since({ ago: ago(since, now) }) : m.apps_never_online(),
      icon: 'ti-plug-connected-x',
    };
  }
  return { kind: 'unknown', label: m.apps_status_unknown(), detail: contactOf(app, now), icon: 'ti-clock' };
}

function contactOf(app: AdminApp, now: number): string {
  const at = app.contacted_at ? Date.parse(app.contacted_at) : NaN;
  return Number.isFinite(at) ? m.apps_contacted({ ago: ago(at, now) }) : '';
}

/** Minutes until `t` ("14 min"), at least one. */
function until(t: number, now: number): string {
  return m.apps_minutes({ n: Math.max(1, Math.ceil((t - now) / 60_000)) });
}

/** Problem of the event delivery, otherwise `''`. */
export function eventsProblem(app: AdminApp, now = Date.now()): string {
  const e = app.events;
  if (e.given_up) return m.apps_events_given_up();
  if (e.failing_since) {
    const since = Date.parse(e.failing_since);
    return m.apps_events_failing({ ago: Number.isFinite(since) ? ago(since, now) : '', n: e.pending });
  }
  return '';
}
