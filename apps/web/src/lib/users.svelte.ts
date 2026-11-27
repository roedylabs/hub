/**
 * Administration › Users against the users API: list from `GET users` (users
 * and invitations), search, filter, sorting, selection with bulk actions
 * (`users/bulk`), inviting (`POST invites`) with a result notice and "Copy
 * link" (`GET invites/:id/link`), actions per row and the details of an
 * expanded user (`GET users/:id`: signed-in devices, recovery codes).
 *
 * The hub's toasts have no undo: role, lock, unlock, removing, revoking an
 * invitation and "Reset multi-factor" change the display immediately and go
 * to the server right away (removing, revoking and the reset after the
 * confirmation). If one fails, the rows return with the error toast. Local
 * changes the server does not know yet also apply to lists loaded in the
 * meantime (`#overlays`); after every action the list loads again. A row
 * that returns gets the values the server confirmed last in request order
 * (`#confirmed`, `#stamp`), never those of a change rejected meanwhile.
 *
 * Request sequencing: only the latest list load writes (`#seq`), and only the
 * latest details request for the row that is still open (`#detailSeq`); an
 * invitation response only closes its own form (`InviteForm.id`); nothing
 * writes state after the page is left (`dispose`), only failed actions still
 * show their error toast. The search filters the loaded list locally.
 *
 * App access (`apps` of every row: slug → role): the expanded row and the
 * invitation form offer "No access" or a role per paired app
 * (`GET admin/apps`); a change in the row goes out at once (`PUT|DELETE
 * users/:id/apps/:slug`, for invitations `PATCH invites/:id` with `apps`)
 * and stays shown over lists loaded meanwhile until its response (`#access`);
 * the invitation form sends only the apps the administrator changed – the
 * others get their default when the invitation is accepted. The bulk action
 * "App access" sends `users/bulk` with `access` once per changed app. A
 * pending app (409 `app_pending`), a role the app no longer has (422) or an
 * app removed meanwhile reload the apps and the list; in the invitation form
 * they show at the app (`fields["apps.<slug>"]`).
 *
 * Domain errors: `invalid` with `fields` (`emails.<i>`, `name`, `emails`
 * `too_many`), `rate_limited` with `retry_after` (inviting: the form locks with
 * the wait time), `mail_failed` on "Resend" (note in the row with "Copy
 * link", for an expired invitation with "Extend"; `POST invites` reports
 * unsent mails per address instead), `last_admin`,
 * `mfa_disabled`, `not_found` (gone in the meantime: toast and reload; for
 * removing it counts as done).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/users.svelte.ts`) without the
 * deferred actions with undo, the list kept for the session (`keptUsers`),
 * the search and invitation shortcuts of the shell and the time zone of the
 * settings; with the details of the expanded row, "Requested a new
 * invitation" (`requested_at`) and the system mail from
 * `GET admin/system-mail` instead of `auth/me`.
 */
import { AV } from '@roedy/tokens';
import { SvelteSet } from 'svelte/reactivity';
import { lostAccess } from '#lib/access.ts';
import { ago } from '#lib/ago.ts';
import { api, fieldsOf, isApiError, retryAfter } from '#lib/api.ts';
import {
  accessOptions,
  appName,
  defaultAccess,
  isPaired,
  type AccessChoice,
  type AccessOption,
  type AdminApp,
} from '#lib/apps.ts';
import { copyPending } from '#lib/clipboard.svelte.ts';
import { Countdown } from '#lib/countdown.svelte.ts';
import { isEmail } from '#lib/email.ts';
import { failText, loadFailText } from '#lib/errors.ts';
import type { IconName } from '#lib/icons.ts';
import type { KeyInput } from '#lib/keys.ts';
import { intlLocale } from '#lib/locale.ts';
import { nameError, nameErrorText, normalizeName } from '#lib/name.ts';
import { m } from '#lib/paraglide/messages.js';
import { pickList, type SelectValues } from '#lib/select.ts';
import { initialsOf, session } from '#lib/state/session.svelte.ts';
import { Loading, ui } from '#lib/state/ui.svelte.ts';
import { waitText } from '#lib/wait.ts';

/** At most this many addresses at once (`POST invites`, `MAX_INVITES` of the server). */
export const MAX_INVITES = 50;

export type Role = 'admin' | 'user';
export type UserStatus = 'active' | 'locked' | 'invited';
export type UsersFilter = 'all' | UserStatus;
export type SortKey = 'name' | 'role' | 'status';
export type ListStatus = 'loading' | 'ready' | 'failed';

/** Entry of `GET users` (`api::users::user_json` or `api::invites::invite_json`). */
export interface ListedUser {
  id: string;
  status: UserStatus;
  /** Display name; for invitations only if it was in the form. */
  name: string | null;
  email: string;
  username?: string | null;
  role: Role;
  /** Confirmed authenticator app (users only). */
  mfa?: boolean;
  last_seen_at?: string | null;
  created_at: string;
  invited_by?: { id: string; name: string | null } | null;
  /** Last successful sending; empty = "Not sent". */
  mail_sent_at?: string | null;
  expires_at?: string;
  expired?: boolean;
  /** The invitee asked for a new invitation (expired invitations, `invites/:token/request`). */
  requested_at?: string | null;
  /** Paired apps with access, slug → role; for invitations what accepting gives now. */
  apps?: Record<string, string>;
}

/** Signed-in device of `GET users/:id` (without IP addresses). */
export interface UserSession {
  created_at: string;
  last_seen_at: string | null;
}

/** What `GET users/:id` adds to the row. */
export interface UserDetails {
  sessions: UserSession[];
  /** Unused and total recovery codes; `null` without codes. */
  recovery_codes: { unused: number; total: number } | null;
}

/** Details of the expanded row. */
export interface DetailsState {
  id: string;
  status: 'loading' | 'ready' | 'failed';
  data: UserDetails | null;
}

/** Response of `POST invites`. */
interface CreatedInvites {
  invites: { id: string; email: string; sent: boolean; recorded?: boolean }[];
  system_mail: boolean;
}

/** Form "Invite people". */
interface InviteForm {
  /** Counts up per opened form: responses only apply to their form. */
  id: number;
  /** Text in the field, not yet a chip. */
  email: string;
  name: string;
  list: string[];
  role: Role;
  focus: boolean;
  busy: boolean;
  /** Name error of the server (`fields.name`) until the name changes. */
  nameErr: 'too_long' | 'invalid' | null;
  /** Access the administrator chose per app; apps not named get their default on acceptance. */
  apps: Record<string, AccessChoice>;
  /** Errors of the server per app (`fields["apps.<slug>"]`) until its choice changes. */
  appErr: Record<string, AppError>;
}

/** Reasons of `fields["apps.<slug>"]` of `POST invites`. */
export type AppError = 'not_found' | 'app_pending' | 'invalid';

/** Bulk action "App access": the choice per app; `undefined` keeps it. */
interface BulkAccess {
  choices: Record<string, AccessChoice | undefined>;
  busy: boolean;
}

/** `apps` of a row with `slug` set to `value` (`null`: no access). */
function withApp(apps: Record<string, string> | undefined, slug: string, value: AccessChoice): Record<string, string> {
  const next = { ...apps };
  if (value === null) delete next[slug];
  else next[slug] = value;
  return next;
}

/** Result after "Send invitation" if not all mails went out. */
interface InviteResult {
  /** Unsent invitations as `POST invites` returned them: shown even before the list has them. */
  invites: { id: string; email: string }[];
  n: number;
  /** No system mail: invitations only created. */
  off: boolean;
  /** A list loaded since: an invitation missing from it was revoked or accepted. */
  listed: boolean;
}

/** Roles (order = sorting "Role"). */
const ROLES: readonly Role[] = ['admin', 'user'];
const STATUS_ORDER: Record<UserStatus, number> = { active: 0, invited: 1, locked: 2 };
const FILTERS: readonly UsersFilter[] = ['all', 'active', 'invited', 'locked'];
/** Fields that actions of the list change. */
type RowPatch = Partial<Pick<ListedUser, 'role' | 'status' | 'mfa'>>;
const patchKeys = (patch: Partial<Record<keyof RowPatch, unknown>>) => Object.keys(patch) as (keyof RowPatch)[];

/**
 * Value of a field the server has, and when it was confirmed: the position (`#stamp`) of the change or list
 * request that reported it. The server applies requests in the order they arrive, so a later position wins.
 */
interface Confirmed {
  value: RowPatch[keyof RowPatch];
  at: number;
}

/** Local change the server does not know yet (call on its way). */
interface Overlay {
  ids: readonly string[];
  /** Changed fields; without: rows removed. */
  patch?: RowPatch;
  /** These of its rows without this change (as the server still knows them). */
  undo: (rows: ListedUser[], ids: readonly string[]) => ListedUser[];
  /** A newer list from the server: the base for `undo`. */
  rebase?: (users: ListedUser[]) => void;
}

/** Lifting a local change: revert it or, if the server knows it, settle it. */
interface Change {
  revert: () => void;
  /** Settle; with `confirmed` only these rows, the others return (what `users/bulk` skipped). */
  settle: (confirmed?: readonly string[]) => void;
}

/** Separators between addresses in the field (comma, semicolon, whitespace). */
const SEP = /[\s,;]+/;

const roleLabel = (r: Role) => (r === 'admin' ? m.users_role_admin() : m.users_role_user());
const roleDesc = (r: Role) => (r === 'admin' ? m.users_role_admin_desc() : m.users_role_user_desc());
const roleIcon = (r: Role): IconName => (r === 'admin' ? 'ti-crown' : 'ti-user');
const statusLabel = (s: UserStatus) =>
  s === 'active' ? m.users_status_active() : s === 'invited' ? m.users_status_invited() : m.users_status_locked();

/** Shown name: name, otherwise the address. */
export const displayName = (u: Pick<ListedUser, 'name' | 'email'>) => u.name || u.email;
const people = (n: number) => (n === 1 ? m.users_people_one() : m.users_people_many({ n }));
/** Who changed for a toast: the name of a single changed row, otherwise the number. */
function whoOf(list: readonly ListedUser[], changed: readonly string[]): string {
  const one = changed.length === 1 ? list.find((u) => u.id === changed[0]) : undefined;
  return one ? displayName(one) : people(changed.length);
}

/** Point in time of the row: for invitations the last sending, for users the last activity. */
function atOf(u: ListedUser): number {
  const iso = u.status === 'invited' ? (u.mail_sent_at ?? u.created_at) : (u.last_seen_at ?? u.created_at);
  return Date.parse(iso) || 0;
}

/** Fixed colour per address (so it does not shift when a row is removed). */
function avatarOf(email: string): readonly [string, string] {
  let h = 7;
  for (const ch of email.toLowerCase()) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return AV[h % AV.length] ?? AV[0];
}

function splitAddresses(value: string): string[] {
  return value.split(SEP).filter(Boolean);
}

/** Append addresses, duplicates regardless of case only once. */
function addAddresses(list: readonly string[], tokens: readonly string[]): string[] {
  const out = [...list];
  for (const raw of tokens) {
    const t = raw.trim();
    if (t && !out.some((x) => x.toLowerCase() === t.toLowerCase())) out.push(t);
  }
  return out;
}

/** Rejects except for 404: something removed in the meantime is gone as intended. */
function goneIsDone(call: Promise<unknown>): Promise<unknown> {
  return call.catch((error: unknown) => {
    if (isApiError(error, 404)) return null;
    throw error;
  });
}

type ChipKind = 'ok' | 'bad' | 'dup';

/** Key in a field (`#lib/keys.ts`). */
export type { KeyInput };

export class UsersPage {
  rows = $state<ListedUser[]>([]);
  status = $state<ListStatus>('loading');
  /** Reason the first load failed (text of the card). */
  failure = $state('');
  /** Skeleton only after the skeleton delay. */
  readonly loading = new Loading();
  /** System mail set up (`GET admin/system-mail`, `system_mail` of `POST invites`); until known: yes. */
  hasMail = $state(true);

  q = $state('');
  filter = $state<UsersFilter>('all');
  sort = $state<SortKey>('name');
  dir = $state<1 | -1>(1);
  /** Expanded row. */
  open = $state<string | null>(null);
  /** Details of the expanded user (`GET users/:id`). */
  details = $state<DetailsState | null>(null);
  selected = $state<string[]>([]);
  invite = $state<InviteForm | null>(null);
  result = $state<InviteResult | null>(null);
  /** "Resend" failed (`mail_failed`). */
  sendFail = $state<string | null>(null);
  /** Marked role of the select list "Change role" (arrow keys). */
  roleHi = $state<number | null>(null);
  /** Rows with a running request (resend, extend). */
  readonly busy = new SvelteSet<string>();
  /** Lock of "Send invitation" after 429 (`invites.mail`); the wait time runs along. */
  readonly inviteLock = new Countdown<'invite'>();
  /** Apps of the administration (`GET admin/apps`); access is given in the paired ones. */
  apps = $state<AdminApp[]>([]);
  paired = $derived(this.apps.filter(isPaired));
  /** Bulk action "App access" while open. */
  bulkAccess = $state<BulkAccess | null>(null);

  /** Addresses reported by the server as taken or invalid (lower case), per form. */
  readonly #taken = new SvelteSet<string>();
  readonly #invalid = new SvelteSet<string>();
  #overlays: Overlay[] = [];
  /**
   * Confirmed values of the fields changes are on their way for, per row; kept while any change (also a removal)
   * of the row is on its way. A change on its way is no baseline: reverting or restoring a row falls back to these.
   */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- bookkeeping, nothing renders it (like `#overlays`)
  readonly #confirmed = new Map<string, Partial<Record<keyof RowPatch, Confirmed>>>();
  /** Counts changes and list requests in the order they go out (`Confirmed.at`). */
  #stamp = 0;
  #seq = 0;
  #detailSeq = 0;
  #formId = 0;
  /** Counts mail states from `POST invites`: a `GET admin/system-mail` sent before writes nothing. */
  #mailRev = 0;
  #disposed = false;
  /** Counts opening and closing rows by the admin (`#restorer`). */
  #openRev = 0;
  /** Counts changes of the selection by the admin (`#restorer`). */
  #selRev = 0;
  #appsSeq = 0;
  /** Access changes on their way per row and app (`<id> <slug>`), shown over loaded lists until their response. */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- bookkeeping, nothing renders it (like `#overlays`)
  readonly #access = new Map<string, { id: string; slug: string; value: AccessChoice; token: number }>();
  #accessToken = 0;

  me = $derived(session.user?.id ?? null);

  /** Hits of the search. */
  matched = $derived.by(() => {
    const q = this.q.trim().toLocaleLowerCase(intlLocale());
    if (!q) return this.rows;
    return this.rows.filter(
      (u) =>
        (u.name ?? '').toLocaleLowerCase(intlLocale()).includes(q) ||
        u.email.toLocaleLowerCase(intlLocale()).includes(q),
    );
  });

  /** Visible rows: filter and sorting. */
  list = $derived.by(() => {
    const byName = (a: ListedUser, b: ListedUser) => displayName(a).localeCompare(displayName(b), intlLocale());
    const cmp: Record<SortKey, (a: ListedUser, b: ListedUser) => number> = {
      name: byName,
      role: (a, b) => ROLES.indexOf(a.role) - ROLES.indexOf(b.role),
      status: (a, b) => STATUS_ORDER[a.status] - STATUS_ORDER[b.status] || atOf(b) - atOf(a),
    };
    return this.matched
      .filter((u) => this.filter === 'all' || u.status === this.filter)
      .sort((a, b) => cmp[this.sort](a, b) * this.dir || byName(a, b));
  });

  /** Selectable: visible and not the own account. */
  pickable = $derived(this.list.filter((u) => u.id !== this.me));
  /** Selected and visible. */
  chosen = $derived(this.pickable.filter((u) => this.selected.includes(u.id)));

  /** All addresses of the list, lower case. */
  #known = $derived(this.rows.map((u) => u.email.toLowerCase()));

  /** Addresses in the form including the text in the field. */
  inviteCount = $derived(this.invite ? addAddresses(this.invite.list, [this.invite.email]).length : 0);

  /** Addresses marked red; locks "Send invitation". */
  chipError = $derived.by(() => {
    const bad = (this.invite?.list ?? []).filter((e) => this.chipKind(e) !== 'ok');
    const first = bad[0];
    if (first === undefined) return '';
    if (bad.length > 1) return m.users_invite_err_many({ n: bad.length });
    return this.chipKind(first) === 'dup'
      ? m.users_invite_err_taken({ email: first })
      : m.users_invite_err_invalid({ email: first });
  });

  /**
   * Message below the form: more than 50 addresses, lock (429), name (only
   * with one address), addresses marked red; locks "Send invitation".
   */
  inviteError = $derived.by(() => {
    const form = this.invite;
    if (!form) return '';
    if (this.inviteCount > MAX_INVITES) return m.users_invite_err_max({ n: MAX_INVITES });
    if (this.inviteLock.active('invite')) return m.error_limit({ wait: waitText(this.inviteLock.left('invite')) });
    const name = this.inviteCount <= 1 ? form.nameErr || nameError(form.name) : '';
    if (name) return nameErrorText(name);
    if (this.paired.some((a) => form.appErr[a.slug])) return m.users_invite_err_apps();
    return this.chipError;
  });

  chipKind(email: string): ChipKind {
    const key = email.toLowerCase();
    if (!isEmail(email) || this.#invalid.has(key)) return 'bad';
    return this.#known.includes(key) || this.#taken.has(key) ? 'dup' : 'ok';
  }

  /** First load of the page: the list and whether a system mail is set up. */
  start(): void {
    void this.load();
    void this.#loadMail();
    void this.loadApps();
  }

  /** Apps for the access choices; on an error none are offered (the rows keep their `apps`). */
  async loadApps(): Promise<void> {
    const seq = ++this.#appsSeq;
    try {
      const { apps } = await api<{ apps: AdminApp[] }>('GET', 'admin/apps');
      if (seq === this.#appsSeq && !this.#disposed) this.apps = apps;
    } catch {
      // Without the apps the page offers no access choices.
    }
  }

  /** Whether invitations go out by mail: `GET admin/system-mail` (`status` `none` = no server). */
  async #loadMail(): Promise<void> {
    const rev = this.#mailRev;
    try {
      const res = await api<{ status: string }>('GET', 'admin/system-mail');
      if (!this.#disposed && rev === this.#mailRev) this.hasMail = res.status !== 'none';
    } catch {
      // Unknown: offer sending; "Resend" then reports `mail_failed`, `POST invites` its `system_mail`.
    }
  }

  /** Loads the list; after the first load it stays as it is on an error. Only the latest load writes. */
  async load(): Promise<void> {
    const seq = ++this.#seq;
    const at = ++this.#stamp;
    try {
      const { users } = await this.loading.run(api<{ users: ListedUser[] }>('GET', 'users'));
      if (seq !== this.#seq || this.#disposed) return;
      this.rows = this.#overlaid(users, at);
      this.status = 'ready';
      // Loads before the invitation write nothing any more (`#seq`): this list knows its invitations.
      if (this.result && !this.result.listed) this.result = { ...this.result, listed: true };
      const ids = this.rows.map((u) => u.id);
      this.selected = this.selected.filter((id) => ids.includes(id));
      if (this.open && !ids.includes(this.open)) this.#close();
    } catch (error) {
      if (seq !== this.#seq || this.#disposed) return;
      // Without a session or role: reload the account, the layout then shows the sign-in or "Access denied".
      lostAccess(error);
      if (this.status !== 'ready') {
        this.failure = loadFailText(error);
        this.status = 'failed';
      } else {
        ui.toasts.error(failText(error, m.users_list_failed()));
      }
    }
  }

  /** Loaded list (requested at `at`) with the local changes the server does not know yet. */
  #overlaid(users: ListedUser[], at: number): ListedUser[] {
    // What the server listed is confirmed, unless a change sent after the list request confirmed something newer.
    for (const u of users) {
      const known = this.#confirmed.get(u.id);
      if (known) for (const key of patchKeys(known)) this.#confirm(u.id, key, u[key], at);
    }
    let rows = users.map((u) => this.#withAccess(u));
    for (const o of this.#overlays) {
      o.rebase?.(users);
      const { patch } = o;
      rows = patch
        ? rows.map((u) => (o.ids.includes(u.id) ? { ...u, ...patch } : u))
        : rows.filter((u) => !o.ids.includes(u.id));
    }
    return rows;
  }

  /** A loaded row with the access changes still on their way. */
  #withAccess(u: ListedUser): ListedUser {
    let apps: Record<string, string> | null = null;
    for (const a of this.#access.values()) if (a.id === u.id) apps = withApp(apps ?? u.apps, a.slug, a.value);
    return apps ? { ...u, apps } : u;
  }

  /** The server has `value` since `at`: it becomes the field's baseline unless something later confirmed one. */
  #confirm(id: string, key: keyof RowPatch, value: RowPatch[keyof RowPatch], at: number): void {
    const known = this.#confirmed.get(id) ?? {};
    const was = known[key];
    if (was && was.at > at) return;
    known[key] = { value, at };
    this.#confirmed.set(id, known);
  }

  /** A row with its confirmed values and the changes still on their way (never one already lifted). */
  #restored(u: ListedUser): ListedUser {
    const row = { ...u };
    for (const [key, c] of Object.entries(this.#confirmed.get(u.id) ?? {})) Object.assign(row, { [key]: c.value });
    for (const o of this.#overlays) if (o.patch && o.ids.includes(u.id)) Object.assign(row, o.patch);
    return row;
  }

  /** Local change until the server knows it (`settle`) or rejects it (`revert`). */
  #change(o: Overlay): Change {
    // Requests go out in this order; the server applies them in it.
    const at = ++this.#stamp;
    this.#overlays.push(o);
    /** Lift the change, bring back these rows without it; forget confirmed values nothing needs any more. */
    const lift = (back: readonly string[]) => {
      this.#overlays = this.#overlays.filter((x) => x !== o);
      if (back.length && !this.#disposed) this.rows = o.undo(this.rows, back);
      for (const id of o.ids) if (!this.#overlays.some((x) => x.ids.includes(id))) this.#confirmed.delete(id);
    };
    return {
      settle: (confirmed = o.ids) => {
        // The server has these values now: the baseline of the other changes of the same fields.
        const { patch } = o;
        if (patch)
          for (const id of confirmed) for (const key of patchKeys(patch)) this.#confirm(id, key, patch[key], at);
        lift(o.ids.filter((id) => !confirmed.includes(id)));
      },
      revert: () => lift(o.ids),
    };
  }

  /** "Try again" of the card: skeleton again until the response. */
  async retry(): Promise<void> {
    this.status = 'loading';
    await this.load();
  }

  /** Leave the page: late responses write nothing any more; failed actions still show their toast. */
  dispose(): void {
    this.#disposed = true;
    this.inviteLock.stop();
  }

  // Search, filter, sorting, selection

  setQuery(value: string): void {
    this.q = value;
  }

  setFilter(filter: UsersFilter): void {
    this.filter = filter;
  }

  /** Reset filter and search. */
  resetFilter(): void {
    this.q = '';
    this.filter = 'all';
  }

  /** Sort by column; the same column again reverses the direction. */
  sortBy(key: SortKey): void {
    this.dir = this.sort === key ? (this.dir === 1 ? -1 : 1) : 1;
    this.sort = key;
  }

  toggleOpen(id: string): void {
    this.#openRev++;
    this.sendFail = null;
    ui.closeMenu();
    if (this.open === id) {
      this.#close();
      return;
    }
    this.open = id;
    const row = this.rows.find((u) => u.id === id);
    if (row && row.status !== 'invited') void this.loadDetails(id);
    else this.#dropDetails();
  }

  /** Close the expanded row; a details request still on its way no longer applies. */
  #close(): void {
    this.open = null;
    this.#dropDetails();
  }

  #dropDetails(): void {
    this.#detailSeq++;
    this.details = null;
  }

  /**
   * Details of the expanded user: devices and recovery codes. Only the latest request writes, and only while
   * its row is still open; known details stay visible while they load again.
   */
  async loadDetails(id: string): Promise<void> {
    const seq = ++this.#detailSeq;
    const known = this.details?.id === id ? this.details.data : null;
    this.details = { id, status: 'loading', data: known };
    try {
      const res = await api<{ user: ListedUser & UserDetails }>('GET', `users/${id}`);
      if (seq !== this.#detailSeq || this.#disposed || this.open !== id) return;
      const { sessions, recovery_codes } = res.user;
      this.details = { id, status: 'ready', data: { sessions, recovery_codes } };
    } catch (error) {
      if (seq !== this.#detailSeq || this.#disposed || this.open !== id) return;
      if (isApiError(error, 404)) {
        // Removed in the meantime (by another administrator): the reload closes the row.
        this.#dropDetails();
        ui.toasts.error(m.users_gone());
        void this.load();
        return;
      }
      this.details = { id, status: 'failed', data: known };
    }
  }

  /** After a change of the expanded user: devices and codes may have changed (lock ends sessions, reset). */
  #refreshDetails(id: string): void {
    if (this.#disposed || this.open !== id) return;
    const row = this.rows.find((u) => u.id === id);
    if (row && row.status !== 'invited') void this.loadDetails(id);
  }

  toggleSelect(id: string): void {
    if (id === this.me) return;
    this.#selRev++;
    this.selected = this.selected.includes(id) ? this.selected.filter((x) => x !== id) : [...this.selected, id];
    if (!this.selected.length) this.bulkAccess = null;
  }

  /** Select all visible ones or, if all are already selected, none. */
  toggleAll(): void {
    const all = this.chosen.length > 0 && this.chosen.length === this.pickable.length;
    this.#selRev++;
    this.selected = all ? [] : this.pickable.map((u) => u.id);
    if (all) this.bulkAccess = null;
  }

  clearSelection(): void {
    this.#selRev++;
    this.selected = [];
    this.bulkAccess = null;
    ui.closeMenu();
  }

  // Inviting

  openInvite(): void {
    const id = ++this.#formId;
    this.invite = {
      id,
      email: '',
      name: '',
      list: [],
      role: 'user',
      focus: false,
      busy: false,
      nameErr: null,
      apps: {},
      appErr: {},
    };
    // What the server reported as taken may have changed since (e.g. a discarded email change).
    this.#taken.clear();
    this.#invalid.clear();
    this.#openRev++;
    this.#close();
    this.#selRev++;
    this.selected = [];
    this.bulkAccess = null;
    this.result = null;
  }

  closeInvite(): void {
    this.invite = null;
  }

  #setForm(patch: Partial<InviteForm>): void {
    if (this.invite) this.invite = { ...this.invite, ...patch };
  }

  /** Input in the address field: separators make chips. */
  inviteInput(value: string): void {
    const form = this.invite;
    if (!form) return;
    if (!SEP.test(value)) {
      this.#setForm({ email: value });
      return;
    }
    const parts = value.split(SEP);
    const last = /[\s,;]$/.test(value) ? '' : (parts.pop() ?? '');
    this.#setForm({ list: addAddresses(form.list, parts), email: last });
  }

  /** Take over the text in the field as a chip. */
  #commitText(): void {
    const form = this.invite;
    if (form) this.#setForm({ list: addAddresses(form.list, splitAddresses(form.email)), email: '' });
  }

  /** Enter takes over or sends, backspace in the empty field removes the last chip, Escape closes. */
  inviteKey(event: KeyInput): void {
    const form = this.invite;
    if (!form) return;
    if (event.key === 'Enter') {
      event.preventDefault();
      if (form.email.trim()) this.#commitText();
      else void this.sendInvites();
    } else if (event.key === 'Backspace' && !form.email && form.list.length) {
      this.#setForm({ list: form.list.slice(0, -1) });
    } else if (event.key === 'Escape') {
      event.preventDefault();
      this.closeInvite();
    }
  }

  inviteFocus(): void {
    this.#setForm({ focus: true });
  }

  inviteBlur(): void {
    this.#commitText();
    this.#setForm({ focus: false });
  }

  inviteName(value: string): void {
    this.#setForm({ name: value, nameErr: null });
  }

  inviteRole(role: Role): void {
    this.#setForm({ role });
  }

  /** Access to an app for the invitation; its error goes. */
  inviteAccess(slug: string, value: AccessChoice): void {
    const form = this.invite;
    if (!form) return;
    const appErr = Object.fromEntries(Object.entries(form.appErr).filter(([s]) => s !== slug));
    this.#setForm({ apps: { ...form.apps, [slug]: value }, appErr });
  }

  removeChip(email: string): void {
    const form = this.invite;
    if (form) this.#setForm({ list: form.list.filter((e) => e !== email) });
  }

  /** "Send invitation" or without system mail "Create invitation". */
  async sendInvites(): Promise<void> {
    const form = this.invite;
    if (!form || form.busy) return;
    const list = addAddresses(form.list, splitAddresses(form.email));
    this.#setForm({ list, email: '' });
    if (!list.length || this.inviteError) return;
    const name = list.length === 1 ? normalizeName(form.name) : '';
    // Only the apps still offered: one removed meanwhile is not sent.
    const apps = Object.fromEntries(
      Object.entries(form.apps).filter(([slug]) => this.paired.some((a) => a.slug === slug)),
    );
    this.#setForm({ busy: true });
    // If the form was closed (and reopened) in the meantime, the response does not apply to the new one.
    const current = () => this.invite?.id === form.id;
    try {
      const res = await api<CreatedInvites>('POST', 'invites', {
        emails: list,
        role: form.role,
        ...(name ? { name } : {}),
        ...(Object.keys(apps).length ? { apps } : {}),
      });
      if (this.#disposed) return;
      // `recorded: false`: sent, but the list still shows "Not sent" – no result notice for it.
      const unsent = res.invites.filter((i) => !i.sent).map(({ id, email }) => ({ id, email }));
      if (current()) this.invite = null;
      this.filter = 'all';
      this.#mailRev++;
      this.hasMail = res.system_mail;
      this.result = unsent.length ? { invites: unsent, n: list.length, off: !res.system_mail, listed: false } : null;
      if (!unsent.length) {
        ui.toasts.show(list.length === 1 ? m.users_invite_sent() : m.users_invite_sent_many({ n: list.length }));
      }
      await this.load();
    } catch (error) {
      if (this.#disposed || !current()) {
        this.#failed(error);
        return;
      }
      this.#setForm({ busy: false });
      this.#inviteFailed(error, list);
    }
  }

  #inviteFailed(error: unknown, list: readonly string[]): void {
    if (isApiError(error, 422, 'invalid')) {
      let known = false;
      const fields = fieldsOf(error);
      for (const [path, detail] of Object.entries(fields)) {
        const i = /^emails\.(\d+)$/.exec(path)?.[1];
        const email = i === undefined ? undefined : list[Number(i)];
        if (email === undefined) continue;
        known = true;
        (detail.code === 'taken' ? this.#taken : this.#invalid).add(email.toLowerCase());
      }
      const name = fields.name?.code;
      if (name === 'too_long' || name === 'invalid') {
        known = true;
        this.#setForm({ nameErr: name });
      }
      const appErr: Record<string, AppError> = {};
      for (const [path, detail] of Object.entries(fields)) {
        const slug = /^apps\.(.+)$/.exec(path)?.[1];
        const code = detail.code;
        if (slug && (code === 'not_found' || code === 'app_pending' || code === 'invalid')) appErr[slug] = code;
      }
      if (Object.keys(appErr).length) {
        known = true;
        this.#setForm({ appErr });
        // The apps changed meanwhile: offer them as they are now.
        void this.loadApps();
      }
      // The form already checks more than 50 addresses; if the server counts differently, the same text.
      if (fields.emails?.code === 'too_many') ui.toasts.error(m.users_invite_err_max({ n: MAX_INVITES }));
      else if (!known) ui.toasts.error(m.error_act());
      return;
    }
    if (isApiError(error, 429)) {
      this.inviteLock.start('invite', retryAfter(error));
      return;
    }
    this.#failed(error);
  }

  closeResult(): void {
    this.result = null;
  }

  // Actions per row

  /**
   * Action failed: error toast with the text of the place (without a connection "No connection.", gone in the
   * meantime, the last administrator, a lock with its wait time), then reload the list; without a session or role
   * reload the account. After leaving the page only the toast.
   */
  #failed(error: unknown, text: string = m.error_act()): void {
    if (isApiError(error, 429)) {
      ui.toasts.error(m.error_limit({ wait: waitText(retryAfter(error)) }));
      return;
    }
    if (isApiError(error, 404)) ui.toasts.error(m.users_gone());
    else if (isApiError(error, 409, 'last_admin')) ui.toasts.error(m.users_last_admin());
    else ui.toasts.error(failText(error, text));
    if (this.#disposed) return;
    if (lostAccess(error)) return;
    void this.load();
  }

  /**
   * Change fields of some rows until the server knows them (`settle`) or they
   * are reverted – then only exactly these fields of these rows.
   */
  #patch(ids: readonly string[], patch: RowPatch): Change {
    for (const u of this.rows) {
      if (!ids.includes(u.id)) continue;
      // A row without a change of this field on its way shows the confirmed value (a change on its way has one).
      for (const key of patchKeys(patch)) if (!this.#confirmed.get(u.id)?.[key]) this.#confirm(u.id, key, u[key], 0);
    }
    this.rows = this.rows.map((u) => (ids.includes(u.id) ? { ...u, ...patch } : u));
    // Back to the confirmed value with the changes still on their way (`settle`/`revert` lift this one first).
    const undo = (rows: ListedUser[], only: readonly string[]) =>
      rows.map((u) => (only.includes(u.id) ? this.#restored(u) : u));
    return this.#change({ ids, patch, undo });
  }

  /** Remove rows (remove, revoke) until the response; `revert` if the server rejects. */
  #drop(ids: readonly string[]): Change {
    // A newer list replaces it: what the server no longer has does not come back.
    let rows = this.rows;
    this.rows = rows.filter((u) => !ids.includes(u.id));
    this.selected = this.selected.filter((id) => !ids.includes(id));
    if (this.open && ids.includes(this.open)) this.#close();
    // Rows come back with their confirmed values: a change of them rejected meanwhile is not shown again.
    const undo = (now: ListedUser[], only: readonly string[]) => {
      const known = now.map((u) => u.id);
      return [
        ...now,
        ...rows.filter((u) => only.includes(u.id) && !known.includes(u.id)).map((u) => this.#restored(u)),
      ];
    };
    const rebase = (users: ListedUser[]) => {
      rows = users;
    };
    return this.#change({ ids, undo, rebase });
  }

  /**
   * Sends a change shown already: on success the toast `done` (or what `done` makes of the response) and a
   * reload; on an error the rows return (`restore`: also selection and expanded row) with the error toast.
   */
  async #send<T>(
    change: Change,
    call: () => Promise<T>,
    done: (res: T) => string | null,
    fail: (error: unknown) => string = () => m.error_act(),
    restore?: () => void,
    confirmed?: (res: T) => readonly string[],
  ): Promise<void> {
    let res: T;
    try {
      res = await call();
    } catch (error) {
      change.revert();
      if (!this.#disposed) restore?.();
      this.#failed(error, fail(error));
      return;
    }
    const ids = confirmed?.(res);
    change.settle(ids);
    // Rows the server skipped are back: their selection and expanded row too.
    if (ids && restore && !this.#disposed) restore();
    const text = done(res);
    if (text) ui.toasts.show(text);
    if (!this.#disposed) void this.load();
  }

  /**
   * Selection and expanded row before a removal, for a failed call (each
   * only if the admin has not changed it since).
   */
  #restorer(ids: readonly string[]): () => void {
    const selected = this.selected.filter((id) => ids.includes(id));
    const open = this.open !== null && ids.includes(this.open) ? this.open : null;
    const rev = this.#openRev;
    const selRev = this.#selRev;
    return () => {
      const known = this.rows.map((u) => u.id);
      if (selRev === this.#selRev) {
        this.selected = [
          ...this.selected,
          ...selected.filter((id) => known.includes(id) && !this.selected.includes(id)),
        ];
      }
      if (open !== null && rev === this.#openRev && known.includes(open)) {
        this.open = open;
        this.#refreshDetails(open);
      }
    };
  }

  /** Role in the expanded row (`PATCH users/:id` or `invites/:id`). */
  setRole(u: ListedUser, role: Role): void {
    if (u.id === this.me || u.role === role) return;
    const change = this.#patch([u.id], { role });
    const path = u.status === 'invited' ? `invites/${u.id}` : `users/${u.id}`;
    void this.#send(
      change,
      () => api('PATCH', path, { role }),
      () => m.users_role_changed(),
      () => m.users_role_failed(),
    );
  }

  /** "Suspend" with confirmation. */
  lock(u: ListedUser): void {
    const name = displayName(u);
    ui.ask({
      title: m.users_lock_title({ who: name }),
      text: m.users_lock_text({ name }),
      icon: 'ti-lock',
      ok: m.users_lock(),
      danger: true,
      run: () => void this.#setStatus([u], 'locked', false),
    });
  }

  unlock(u: ListedUser): void {
    void this.#setStatus([u], 'active', false);
  }

  /**
   * Lock or unlock; the selection via `users/bulk`, a single row via its resource. The toast names the changed
   * ones; the expanded row's details load again if it changed (locking ends its sessions).
   */
  async #setStatus(list: readonly ListedUser[], status: 'active' | 'locked', bulk: boolean) {
    const ids = list.map((u) => u.id);
    const change = this.#patch(ids, { status });
    const action = status === 'locked' ? 'lock' : 'unlock';
    const text = (changed: readonly string[]) => {
      const who = whoOf(list, changed);
      return status === 'locked' ? m.users_locked({ who }) : m.users_unlocked({ who });
    };
    const many = ids.length > 1;
    const failMsg =
      status === 'locked'
        ? many
          ? m.users_lock_failed_many()
          : m.users_lock_failed()
        : many
          ? m.users_unlock_failed_many()
          : m.users_unlock_failed();
    if (bulk) {
      const changed = await this.#sendBulk(change, { action, ids }, text, failMsg);
      if (this.open && changed.includes(this.open)) this.#refreshDetails(this.open);
      return;
    }
    const [one] = ids;
    await this.#send(
      change,
      () => api('POST', `users/${one}/${action}`),
      () => text(ids),
      () => failMsg,
    );
    if (one) this.#refreshDetails(one);
  }

  /** "Remove" or "Revoke invitation" after confirmation. */
  remove(u: ListedUser): void {
    const name = displayName(u);
    const invited = u.status === 'invited';
    ui.ask({
      title: invited ? m.users_revoke_title() : m.users_remove_title({ who: name }),
      text: invited ? m.users_revoke_text({ email: u.email }) : m.users_remove_text({ name, app: session.appName }),
      icon: 'ti-user-minus',
      ok: invited ? m.users_revoke_ok() : m.users_remove(),
      danger: true,
      run: () => {
        const restore = this.#restorer([u.id]);
        const change = this.#drop([u.id]);
        const path = invited ? `invites/${u.id}` : `users/${u.id}`;
        void this.#send(
          change,
          () => goneIsDone(api('DELETE', path)),
          () => (invited ? m.users_revoked() : m.users_removed({ who: name })),
          undefined,
          restore,
        );
      },
    });
  }

  /** "Reset multi-factor" after confirmation. */
  resetMfa(u: ListedUser): void {
    const name = displayName(u);
    ui.ask({
      title: m.users_mfa_title({ name }),
      text: m.users_mfa_text({ name }),
      icon: 'ti-shield-off',
      ok: m.users_mfa_ok(),
      danger: true,
      run: () => {
        const change = this.#patch([u.id], { mfa: false });
        void this.#send(
          change,
          () => api('POST', `users/${u.id}/reset-mfa`),
          () => m.users_mfa_done({ name }),
          // No app any more (reset elsewhere in the meantime): the reload shows it.
          (error) => (isApiError(error, 409, 'mfa_disabled') ? m.users_mfa_none({ name }) : m.error_act()),
        ).then(() => this.#refreshDetails(u.id));
      },
    });
  }

  /** Replaces an invitation's row with the server's (resend, extend), keeping fields of a change on its way. */
  #replace(id: string, now: ListedUser | null): void {
    if (!now || this.#disposed) return;
    const patch = Object.assign(
      {},
      ...this.#overlays.filter((o) => o.ids.includes(id)).map((o) => o.patch),
    ) as RowPatch;
    this.rows = this.rows.map((r) => (r.id === id ? { ...now, ...patch } : r));
  }

  /** "Resend": the same link; if sending fails (`mail_failed`), a message in the row. */
  async resend(u: ListedUser): Promise<void> {
    if (this.busy.has(u.id)) return;
    this.busy.add(u.id);
    try {
      const res = await api<{ invite: ListedUser | null }>('POST', `invites/${u.id}/resend`);
      if (this.#disposed) return;
      this.#replace(u.id, res.invite);
      if (this.sendFail === u.id) this.sendFail = null;
      ui.toasts.show(m.users_resent());
      void this.load();
    } catch (error) {
      if (isApiError(error, 502, 'mail_failed') && !this.#disposed) this.sendFail = u.id;
      else this.#failed(error);
    } finally {
      this.busy.delete(u.id);
    }
  }

  /** "Extend" an expired invitation: seven days with the same link, without mail and without confirmation. */
  async extend(u: ListedUser): Promise<void> {
    if (this.busy.has(u.id)) return;
    this.busy.add(u.id);
    try {
      const res = await api<{ invite: ListedUser | null }>('POST', `invites/${u.id}/renew`);
      if (this.#disposed) return;
      const now = res.invite;
      this.#replace(u.id, now);
      // A failed "Resend" before no longer applies: the invitation is valid again.
      if (this.sendFail === u.id) this.sendFail = null;
      const until = now?.expires_at ?? null;
      ui.toasts.show(m.users_extended({ date: until ? dayMonth(Date.parse(until)) : '' }));
      void this.load();
    } catch (error) {
      this.#failed(error);
    } finally {
      this.busy.delete(u.id);
    }
  }

  /**
   * "Copy link": the link from the mail, even if it never went out. The link is requested within the click
   * gesture, otherwise Safari refuses the clipboard.
   */
  async copyLink(u: Pick<ListedUser, 'id'>): Promise<void> {
    const link = api<{ url: string }>('GET', `invites/${u.id}/link`).then(({ url }) => url);
    try {
      if (await copyPending(link)) ui.toasts.show(m.users_link_copied());
      else ui.toasts.error(m.error_copy());
    } catch (error) {
      this.#failed(error);
    }
  }

  // App access

  /** Shows `value` as the row's access to `slug`. */
  #setApp(id: string, slug: string, value: AccessChoice): void {
    this.rows = this.rows.map((u) => (u.id === id ? { ...u, apps: withApp(u.apps, slug, value) } : u));
  }

  /**
   * Access of one row to one app, at once: users via `users/:id/apps/:slug`, invitations via `PATCH invites/:id`.
   * The row shows the choice until the response; a later change of the same app wins over an earlier response,
   * which then reports nothing.
   */
  setAccess(u: ListedUser, slug: string, value: AccessChoice): void {
    const app = this.paired.find((a) => a.slug === slug);
    const shown = this.rows.find((r) => r.id === u.id);
    const before = shown?.apps?.[slug] ?? null;
    if (!app || !shown || before === value) return;
    const key = `${u.id} ${slug}`;
    const token = ++this.#accessToken;
    this.#access.set(key, { id: u.id, slug, value, token });
    this.#setApp(u.id, slug, value);
    const call: Promise<ListedUser | null> =
      u.status === 'invited'
        ? api<{ invite: ListedUser | null }>('PATCH', `invites/${u.id}`, { apps: { [slug]: value } }).then(
            (r) => r.invite,
          )
        : value === null
          ? api<{ user: ListedUser }>('DELETE', `users/${u.id}/apps/${slug}`).then((r) => r.user)
          : api<{ user: ListedUser }>('PUT', `users/${u.id}/apps/${slug}`, { app_role: value }).then((r) => r.user);
    const name = appName(app);
    void call.then(
      (row) => {
        if (this.#access.get(key)?.token !== token) return;
        this.#access.delete(key);
        if (!this.#disposed) this.#setApp(u.id, slug, row?.apps?.[slug] ?? value);
        ui.toasts.show(value === null ? m.users_access_removed({ app: name }) : m.users_access_changed({ app: name }));
      },
      (error: unknown) => {
        // A later change of the same app decides; it reports its own result.
        if (this.#access.get(key)?.token !== token) return;
        this.#access.delete(key);
        if (!this.#disposed) this.#setApp(u.id, slug, before);
        this.#accessFailed(error);
      },
    );
  }

  /** A change of access failed: the app is pending, the role or the app is gone – reload apps and list. */
  #accessFailed(error: unknown): void {
    if (isApiError(error, 409, 'app_pending') || isApiError(error, 422) || isApiError(error, 404)) {
      ui.toasts.error(
        isApiError(error, 409)
          ? m.users_access_pending()
          : isApiError(error, 422)
            ? m.users_access_role_gone()
            : m.users_access_gone(),
      );
      if (this.#disposed) return;
      void this.loadApps();
      void this.load();
      return;
    }
    this.#failed(error, m.users_access_failed());
  }

  openBulkAccess(): void {
    ui.closeMenu();
    this.bulkAccess = this.bulkAccess ? null : { choices: {}, busy: false };
  }

  closeBulkAccess(): void {
    this.bulkAccess = null;
  }

  bulkChoose(slug: string, value: AccessChoice | undefined): void {
    const b = this.bulkAccess;
    if (b && !b.busy) this.bulkAccess = { ...b, choices: { ...b.choices, [slug]: value } };
  }

  /** "Apply": `users/bulk` with `access` per changed app, one after another; then the list loads again. */
  async applyBulkAccess(): Promise<void> {
    const b = this.bulkAccess;
    const ids = this.chosen.map((u) => u.id);
    const changes = Object.entries(b?.choices ?? {}).filter(
      (e): e is [string, AccessChoice] => e[1] !== undefined && this.paired.some((a) => a.slug === e[0]),
    );
    if (!b || b.busy || !ids.length || !changes.length) return;
    this.bulkAccess = { ...b, busy: true };
    const changed: string[] = [];
    try {
      for (const [slug, value] of changes) {
        const body = { action: 'access', ids, slug, access: value !== null, ...(value ? { app_role: value } : {}) };
        const res = await api<{ changed: string[] }>('POST', 'users/bulk', body);
        for (const id of res.changed) if (!changed.includes(id)) changed.push(id);
      }
    } catch (error) {
      if (!this.#disposed && this.bulkAccess) this.bulkAccess = { ...this.bulkAccess, busy: false };
      this.#accessFailed(error);
      return;
    }
    if (changed.length) ui.toasts.show(m.users_access_changed_for({ who: whoOf(this.chosen, changed) }));
    else ui.toasts.error(m.users_bulk_none());
    if (this.#disposed) return;
    this.bulkAccess = null;
    void this.load();
  }

  // Bulk actions of the selection

  /**
   * `users/bulk`: what does not fit (the last administrator, a locked account for "lock" …) stays unchanged
   * and its rows return before the reload; the toast names the changed ones, without any "Nothing was changed".
   * Returns the changed ids (none on an error).
   */
  async #sendBulk(
    change: Change,
    body: { action: 'role' | 'lock' | 'unlock' | 'remove'; ids: string[]; role?: Role },
    text: (changed: readonly string[]) => string,
    fail: string,
    restore?: () => void,
  ): Promise<readonly string[]> {
    let changed: readonly string[] = [];
    await this.#send(
      change,
      () => api<{ changed: string[] }>('POST', 'users/bulk', body),
      (res) => {
        if (res.changed.length > 0) return text(res.changed);
        ui.toasts.error(m.users_bulk_none());
        return null;
      },
      () => fail,
      restore,
      (res) => (changed = res.changed),
    );
    return changed;
  }

  /** Role for the selection; only rows with another role change. */
  bulkRole(role: Role): void {
    ui.closeMenu();
    this.roleHi = null;
    const list = this.chosen.filter((u) => u.role !== role);
    if (!list.length) return;
    const ids = list.map((u) => u.id);
    const change = this.#patch(ids, { role });
    void this.#sendBulk(
      change,
      { action: 'role', ids, role },
      ({ length: n }) => (n === 1 ? m.users_role_changed() : m.users_role_changed_for({ who: people(n) })),
      m.users_role_failed(),
    );
  }

  bulkLock(): void {
    const list = this.chosen.filter((u) => u.status === 'active');
    const [one] = list;
    if (!one) return;
    const who = list.length === 1 ? displayName(one) : people(list.length);
    ui.ask({
      title: m.users_lock_title({ who }),
      text: list.length === 1 ? m.users_bulk_lock_text_one({ name: displayName(one) }) : m.users_bulk_lock_text_many(),
      icon: 'ti-lock',
      ok: m.users_lock(),
      danger: true,
      run: () => void this.#setStatus(list, 'locked', true),
    });
  }

  bulkUnlock(): void {
    const list = this.chosen.filter((u) => u.status === 'locked');
    if (list.length) void this.#setStatus(list, 'active', true);
  }

  /** Remove the selection or revoke invitations after confirmation. */
  bulkRemove(): void {
    const list = this.chosen;
    const [one] = list;
    if (!one) return;
    const single = list.length === 1;
    const invited = single && one.status === 'invited';
    const who = single ? displayName(one) : people(list.length);
    ui.ask({
      title: invited ? m.users_revoke_title() : m.users_remove_title({ who }),
      text: single
        ? invited
          ? m.users_revoke_text({ email: one.email })
          : m.users_remove_text({ name: who, app: session.appName })
        : m.users_bulk_remove_text({ app: session.appName }),
      icon: 'ti-user-minus',
      ok: invited ? m.users_revoke_ok() : m.users_remove(),
      danger: true,
      run: () => {
        const ids = list.map((u) => u.id);
        const restore = this.#restorer(ids);
        const change = this.#drop(ids);
        void this.#sendBulk(
          change,
          { action: 'remove', ids },
          // The selection bar always says "removed", for a single invitation with the address.
          (changed) => m.users_removed({ who: whoOf(list, changed) }),
          m.error_act(),
          restore,
        );
      },
    });
  }
}

// Values of the page

/** Role as a card. */
export interface RoleCard {
  label: string;
  desc: string;
  icon: IconName;
  on: boolean;
  /** Own account: not changeable. */
  dis: boolean;
  onClick: () => void;
}

export interface Chip {
  label: string;
  /** Invalid or taken: red with icon. */
  bad: boolean;
  tip: string | undefined;
  rmLabel: string;
  remove: () => void;
}

export interface Pill {
  label: string;
  n: number;
  on: boolean;
  onClick: () => void;
}

export interface SortHead {
  label: string;
  aria: string;
  on: boolean;
  /** Descending (arrow rotated). */
  desc: boolean;
  onClick: () => void;
}

/** Select list "Change role" without a value on the button. */
export type RolePick = Omit<SelectValues, 'label' | 'disabled'>;

/** Facts of the expanded user's details (`GET users/:id`). */
export interface DetailValues {
  /** Request running without earlier details. */
  loading: boolean;
  failed: boolean;
  /** "Signed in on 2 devices · last active 5 min ago". */
  devices: string;
  /** Recovery codes (only with multi-factor): "8 of 10 recovery codes left". */
  codes: string;
  retry: () => void;
}

/** Access to one app: the choices with the current one checked. */
export interface AccessRow {
  slug: string;
  name: string;
  icon: string | null;
  /** Name of the group: "Access to Mail". */
  label: string;
  options: AccessOption[];
  pick: (value: AccessChoice) => void;
  /** Error at the app (invitation form), otherwise `''`. */
  err: string;
}

/** Access to one app in the bulk action, with "Unchanged". */
export interface BulkAccessRow extends AccessRow {
  unchanged: { on: boolean; pick: () => void };
}

export interface BulkAccessValues {
  title: string;
  rows: BulkAccessRow[];
  busy: boolean;
  /** Something to apply. */
  canApply: boolean;
  apply: () => void;
  cancel: () => void;
}

export interface UserRow {
  id: string;
  name: string;
  email: string;
  sub: string;
  initials: string;
  /** Avatar: background and text colour. */
  av: readonly [string, string];
  isMe: boolean;
  notMe: boolean;
  online: boolean;
  role: string;
  roleIcon: IconName;
  mfa: boolean;
  st: string;
  status: UserStatus;
  /** Invitation expired: status "Expired" in `--danger`. */
  expired: boolean;
  isLockedSt: boolean;
  notLockedSt: boolean;
  last: string;
  lastLong: string;
  sel: boolean;
  isSel: boolean;
  notSel: boolean;
  ckLabel: string;
  open: boolean;
  sendErr: boolean;
  noSendErr: boolean;
  canResend: boolean;
  /** "Copy link": valid invitations; expired ones only after "Extend" or "Resend". */
  canCopy: boolean;
  /** "Extend": only expired invitations, with and without system emails. */
  canExtend: boolean;
  /** Valid invitation: row "Valid until ‹date›". */
  hasValid: boolean;
  validUntil: string;
  hasSendNote: boolean;
  sendNote: string;
  /** Expired invitation whose invitee asked for a new one. */
  requested: boolean;
  roles: RoleCard[];
  isInvited: boolean;
  isActive: boolean;
  isLocked: boolean;
  removeLabel: string;
  canMfaReset: boolean;
  /** Details of the expanded user; `null` for invitations and closed rows. */
  details: DetailValues | null;
  /** Access per paired app of the expanded row; empty for closed rows or without paired apps. */
  access: AccessRow[];
  /** Request running (resend, extend). */
  busy: boolean;
  togSel: () => void;
  toggle: () => void;
  editMe: () => void;
  copyLink: () => void;
  resend: () => void;
  extend: () => void;
  lock: () => void;
  unlock: () => void;
  mfaReset: () => void;
  remove: () => void;
}

export interface InviteResultValues {
  show: boolean;
  /** Without system mail (`ti-mail-off`, calm colours), otherwise a warning. */
  off: boolean;
  icon: IconName;
  title: string;
  text: string;
  rows: { id: string; email: string; copy: () => void }[];
  close: () => void;
}

export interface UsersValues {
  uQ: string;
  uHasQ: boolean;
  onUQ: (value: string) => void;
  clearUQ: () => void;
  uNoMail: boolean;
  uHasMail: boolean;
  /** "Set up email": Administration › Email. */
  uNoMailGo: () => void;
  inviteOpen: boolean;
  inviteClosed: boolean;
  openInvite: () => void;
  invEmail: string;
  invName: string;
  invNameDis: boolean;
  invNamePh: string;
  invNamePhM: string;
  invPh: string;
  invChips: Chip[];
  invFocus: boolean;
  invRoles: RoleCard[];
  /** Access per paired app, preset from each app's default. */
  invApps: AccessRow[];
  invErr: string;
  hasInvErr: boolean;
  invBtn: { dis: boolean };
  invBusy: boolean;
  invSendLabel: string;
  invSendShort: string;
  onInvEmail: (value: string) => void;
  onInvName: (value: string) => void;
  invKey: (event: KeyInput) => void;
  invNameKey: (event: KeyInput) => void;
  onInvFocus: () => void;
  onInvBlur: () => void;
  invSend: () => void;
  invCancel: () => void;
  invRes: InviteResultValues;
  uPills: Pill[];
  uSelOn: boolean;
  uSelOff: boolean;
  uSelText: string;
  uSelClear: () => void;
  selBulkRole: RolePick;
  uBulkCanLock: boolean;
  uBulkCanUnlock: boolean;
  uBulkLock: () => void;
  uBulkUnlock: () => void;
  uBulkRemove: () => void;
  /** Paired apps exist: offer "App access". */
  uCanBulkAccess: boolean;
  uBulkAccessOpen: boolean;
  uBulkAccessToggle: () => void;
  uBulkAccess: BulkAccessValues | null;
  uHasRows: boolean;
  uAllCk: { on: boolean | 'mixed'; label: string; icon: IconName; vis: number; onClick: () => void };
  uHdName: SortHead;
  uHdRole: SortHead;
  uHdStatus: SortHead;
  uRows: UserRow[];
  uEmpty: boolean;
  uEmptyText: string;
  uCanReset: boolean;
  uReset: () => void;
}

export interface UsersContext {
  /** "Edit your account": to Account › Profile. */
  editMe: () => void;
  /** "Set up email": to Administration › Email. */
  setupMail: () => void;
}

function roleCards(current: Role, disabled: boolean, choose: (role: Role) => void): RoleCard[] {
  return ROLES.map((r) => ({
    label: roleLabel(r),
    desc: roleDesc(r),
    icon: roleIcon(r),
    on: r === current,
    dis: disabled,
    onClick: () => choose(r),
  }));
}

function appErrorText(code: AppError): string {
  return code === 'not_found'
    ? m.users_app_err_not_found()
    : code === 'app_pending'
      ? m.users_app_err_pending()
      : m.users_app_err_invalid();
}

function accessRow(app: AdminApp, current: AccessChoice, pick: (value: AccessChoice) => void, err = ''): AccessRow {
  const name = appName(app);
  return {
    slug: app.slug,
    name,
    icon: app.icon,
    label: m.users_access_to({ name }),
    options: accessOptions(app, current),
    pick,
    err,
  };
}

/** Day and month in words ("2 November") in the browser's time zone. */
function dayMonth(t: number): string {
  return new Date(t).toLocaleDateString(intlLocale(), { day: 'numeric', month: 'long' });
}

/** Expiry of an invitation in ms; expired as soon as it has passed (`expired` of the API only applies on loading). */
function expiryOf(u: ListedUser): number | null {
  return u.status === 'invited' && u.expires_at ? Date.parse(u.expires_at) : null;
}

function sendNoteOf(p: UsersPage, u: ListedUser, sendErr: boolean, expired: boolean, now: number): string {
  if (u.status !== 'invited') return '';
  // A failed "Resend" leaves an expired invitation expired: its link no longer works, "Extend" does.
  if (expired && p.sendFail === u.id) return m.users_note_resend_failed_expired();
  if (expired) {
    if (u.requested_at) {
      const when = ago(Date.parse(u.requested_at) || now, now);
      return p.hasMail ? m.users_note_requested({ ago: when }) : m.users_note_requested_off({ ago: when });
    }
    return p.hasMail ? m.users_note_expired() : m.users_note_expired_off();
  }
  if (!p.hasMail && sendErr) return m.users_note_off();
  if (p.sendFail === u.id) return m.users_note_resend_failed();
  return sendErr ? m.users_note_failed() : '';
}

/** Devices and recovery codes of the expanded user. */
function detailValues(p: UsersPage, u: ListedUser, now: number): DetailValues | null {
  const d = p.details;
  if (u.status === 'invited' || !d || d.id !== u.id) return null;
  const data = d.data;
  let devices = '';
  let codes = '';
  if (data) {
    const n = data.sessions.length;
    const last = Math.max(0, ...data.sessions.map((s) => Date.parse(s.last_seen_at ?? s.created_at) || 0));
    devices =
      n === 0
        ? m.users_devices_none()
        : n === 1
          ? m.users_devices_one({ ago: ago(last, now) })
          : m.users_devices_many({ n, ago: ago(last, now) });
    const rc = data.recovery_codes;
    if (u.mfa) codes = rc ? m.users_codes_left({ unused: rc.unused, total: rc.total }) : m.users_codes_none();
  }
  return {
    loading: d.status === 'loading' && !data,
    failed: d.status === 'failed',
    devices,
    codes,
    retry: () => void p.loadDetails(u.id),
  };
}

function rowValues(p: UsersPage, u: ListedUser, ctx: UsersContext, now: number): UserRow {
  const isMe = u.id === p.me;
  const name = displayName(u);
  const sel = !isMe && p.selected.includes(u.id);
  const open = p.open === u.id;
  const invited = u.status === 'invited';
  const sendErr = invited && !u.mail_sent_at;
  const when = ago(atOf(u), now);
  const exp = expiryOf(u);
  const expired = invited && (exp === null ? !!u.expired : exp <= now);
  const until = exp === null ? '' : dayMonth(exp);
  const requested = expired && !!u.requested_at;
  const mfa = !!u.mfa;
  return {
    id: u.id,
    name,
    email: u.email,
    sub: u.name ? u.email : m.users_pending(),
    initials: initialsOf(name),
    av: avatarOf(u.email),
    isMe,
    notMe: !isMe,
    online: isMe,
    role: roleLabel(u.role),
    roleIcon: roleIcon(u.role),
    mfa,
    st: expired ? m.users_expired() : statusLabel(u.status),
    status: u.status,
    expired,
    isLockedSt: u.status === 'locked',
    notLockedSt: u.status !== 'locked',
    last: isMe
      ? m.users_last_now()
      : requested
        ? m.users_requested()
        : expired
          ? m.users_expired_on({ date: until })
          : u.status === 'locked'
            ? m.users_last_locked({ ago: when })
            : when,
    lastLong: isMe
      ? m.users_last_now_long()
      : expired
        ? m.users_expired_long({ date: until })
        : invited
          ? m.users_invited_long({ ago: when })
          : m.users_last_long({ ago: when }),
    sel,
    isSel: sel,
    notSel: !sel,
    ckLabel: isMe ? m.users_check_me() : sel ? m.users_check_deselect({ name }) : m.users_check_select({ name }),
    open,
    sendErr,
    noSendErr: !sendErr,
    canResend: invited && p.hasMail,
    canCopy: invited && !expired,
    canExtend: expired,
    hasValid: invited && !expired && until !== '',
    validUntil: invited && !expired && until !== '' ? m.users_valid_until({ date: until }) : '',
    hasSendNote: invited && (expired || sendErr || p.sendFail === u.id),
    sendNote: sendNoteOf(p, u, sendErr, expired, now),
    requested,
    roles: open ? roleCards(u.role, isMe, (r) => p.setRole(u, r)) : [],
    isInvited: invited,
    isActive: u.status === 'active' && !isMe,
    isLocked: u.status === 'locked',
    removeLabel: invited ? m.users_revoke() : m.users_remove(),
    canMfaReset: mfa && !isMe && !invited,
    details: open ? detailValues(p, u, now) : null,
    access: open ? p.paired.map((a) => accessRow(a, u.apps?.[a.slug] ?? null, (v) => p.setAccess(u, a.slug, v))) : [],
    busy: p.busy.has(u.id),
    togSel: () => p.toggleSelect(u.id),
    toggle: () => p.toggleOpen(u.id),
    editMe: ctx.editMe,
    copyLink: () => void p.copyLink(u),
    resend: () => void p.resend(u),
    extend: () => void p.extend(u),
    lock: () => p.lock(u),
    unlock: () => p.unlock(u),
    mfaReset: () => p.resetMfa(u),
    remove: () => p.remove(u),
  };
}

function inviteResult(p: UsersPage): InviteResultValues {
  const r = p.result;
  // Rows disappear after a successful resend, revocation or acceptance; until a list has loaded since,
  // the invitations from `POST invites` stand for themselves.
  const open = (r?.invites ?? []).filter((i) => {
    const u = p.rows.find((x) => x.id === i.id);
    return u ? u.status === 'invited' && !u.mail_sent_at : !r?.listed;
  });
  const failed = r?.invites.length ?? 0;
  const sent = r ? r.n - failed : 0;
  const one = failed === 1;
  let title = '';
  let text = '';
  if (r?.off) {
    title = r.n === 1 ? m.users_res_created() : m.users_res_created_many({ n: r.n });
    text = one ? m.users_res_off_one() : m.users_res_off_many();
  } else if (r) {
    title = !sent
      ? one
        ? m.users_res_failed()
        : m.users_res_failed_many({ n: failed })
      : m.users_res_partial({
          sent: sent === 1 ? m.users_res_sent_one() : m.users_res_sent_many({ n: sent }),
          unsent: one ? m.users_res_unsent_one() : m.users_res_unsent_many({ n: failed }),
        });
    text = one ? m.users_res_text_one() : m.users_res_text_many();
  }
  return {
    show: open.length > 0,
    off: !!r?.off,
    icon: r?.off ? 'ti-mail-off' : 'ti-alert-triangle',
    title,
    text,
    rows: open.map((i) => ({ id: i.id, email: i.email, copy: () => void p.copyLink(i) })),
    close: () => p.closeResult(),
  };
}

function bulkAccessValues(p: UsersPage): BulkAccessValues | null {
  const b = p.bulkAccess;
  if (!b) return null;
  const rows = p.paired.map((a): BulkAccessRow => {
    const choice = b.choices[a.slug];
    return {
      ...accessRow(a, choice ?? null, (v) => p.bulkChoose(a.slug, v)),
      unchanged: { on: choice === undefined, pick: () => p.bulkChoose(a.slug, undefined) },
    };
  });
  return {
    title: m.users_access_bulk_title({ who: people(p.chosen.length) }),
    rows,
    busy: b.busy,
    canApply: !b.busy && rows.some((r) => !r.unchanged.on),
    apply: () => void p.applyBulkAccess(),
    cancel: () => p.closeBulkAccess(),
  };
}

function rolePick(p: UsersPage): RolePick {
  const n = p.chosen.length;
  const first = p.chosen[0];
  const common = first && p.chosen.every((u) => u.role === first.role) ? first.role : null;
  const open = ui.menu === 'pick';
  return {
    title: m.users_role_for({ who: people(n) }),
    open,
    toggle: () => {
      if (n === 0) return;
      p.roleHi = null;
      ui.toggleMenu('pick');
    },
    ...pickList({
      opts: ROLES.map((r) => ({ v: r, l: roleLabel(r), sub: roleDesc(r) })),
      cur: common,
      open,
      hi: p.roleHi,
      disabled: n === 0,
      mark: (i) => {
        p.roleHi = i;
      },
      show: () => {
        ui.menu = 'pick';
      },
      hide: () => ui.closeMenu(),
      choose: (role) => p.bulkRole(role),
    }),
  };
}

function sortHead(p: UsersPage, key: SortKey, label: string): SortHead {
  const on = p.sort === key;
  return {
    label,
    aria: m.users_sort_by({ label }),
    on,
    desc: on && p.dir === -1,
    onClick: () => p.sortBy(key),
  };
}

/**
 * Rows of the list, derivable separately from the other values: they
 * do not depend on the form, the search in the field or the open menu.
 */
export function rowsValues(p: UsersPage, ctx: UsersContext, now = Date.now()): UserRow[] {
  return p.list.map((u) => rowValues(p, u, ctx, now));
}

/** Values of the page from the state; `rows` from `rowsValues()` if already derived. */
export function usersValues(
  p: UsersPage,
  ctx: UsersContext,
  now = Date.now(),
  rows: UserRow[] = rowsValues(p, ctx, now),
): UsersValues {
  const form = p.invite;
  const n = p.inviteCount;
  const many = n > 1;
  const noMail = !p.hasMail;
  const err = p.inviteError;
  const nSel = p.chosen.length;
  const all: boolean | 'mixed' = nSel === 0 ? false : nSel === p.pickable.length ? true : 'mixed';
  const q = p.q.trim();
  const counts = (f: UsersFilter) => p.matched.filter((u) => f === 'all' || u.status === f).length;
  return {
    uQ: p.q,
    uHasQ: p.q.length > 0,
    onUQ: (value) => p.setQuery(value),
    clearUQ: () => p.setQuery(''),
    uNoMail: noMail,
    uHasMail: !noMail,
    uNoMailGo: ctx.setupMail,
    inviteOpen: !!form,
    inviteClosed: !form,
    openInvite: () => p.openInvite(),
    invEmail: form?.email ?? '',
    invName: form && !many ? form.name : '',
    invNameDis: many,
    invNamePh: many ? m.users_invite_name_placeholder_many() : m.users_invite_name_placeholder(),
    invNamePhM: many ? m.users_invite_name_placeholder_mobile_many() : m.users_invite_name_placeholder_mobile(),
    invPh: form?.list.length ? '' : m.users_invite_email_placeholder(),
    invChips: (form?.list ?? []).map((e) => {
      const kind = p.chipKind(e);
      return {
        label: e,
        bad: kind !== 'ok',
        tip: kind === 'dup' ? m.users_chip_taken() : kind === 'bad' ? m.users_chip_invalid() : undefined,
        rmLabel: m.users_chip_remove({ email: e }),
        remove: () => p.removeChip(e),
      };
    }),
    invFocus: !!form?.focus,
    invRoles: roleCards(form?.role ?? 'user', false, (r) => p.inviteRole(r)),
    invApps: form
      ? p.paired.map((a) => {
          const chosen = Object.hasOwn(form.apps, a.slug) ? (form.apps[a.slug] ?? null) : defaultAccess(a);
          const err = form.appErr[a.slug];
          return accessRow(a, chosen, (v) => p.inviteAccess(a.slug, v), err ? appErrorText(err) : '');
        })
      : [],
    invErr: err,
    hasInvErr: !!err,
    invBtn: { dis: n === 0 || !!err || !!form?.busy },
    invBusy: !!form?.busy,
    invSendLabel: many
      ? noMail
        ? m.users_invite_create_many({ n })
        : m.users_invite_send_many({ n })
      : noMail
        ? m.users_invite_create()
        : m.users_invite_send(),
    invSendShort: many
      ? noMail
        ? m.users_invite_create_short_many({ n })
        : m.users_invite_send_short_many({ n })
      : noMail
        ? m.users_invite_create_short()
        : m.users_invite_send_short(),
    onInvEmail: (value) => p.inviteInput(value),
    onInvName: (value) => p.inviteName(value),
    invKey: (event) => p.inviteKey(event),
    invNameKey: (event) => {
      if (event.key === 'Enter') {
        event.preventDefault();
        void p.sendInvites();
      }
    },
    onInvFocus: () => p.inviteFocus(),
    onInvBlur: () => p.inviteBlur(),
    invSend: () => void p.sendInvites(),
    invCancel: () => p.closeInvite(),
    invRes: inviteResult(p),
    uPills: FILTERS.map((f) => ({
      label: f === 'all' ? m.users_filter_all() : statusLabel(f),
      n: counts(f),
      on: p.filter === f,
      onClick: () => p.setFilter(f),
    })),
    uSelOn: nSel > 0,
    uSelOff: nSel === 0,
    uSelText: m.users_sel_count({ n: nSel }),
    uSelClear: () => p.clearSelection(),
    selBulkRole: rolePick(p),
    uBulkCanLock: p.chosen.some((u) => u.status === 'active'),
    uBulkCanUnlock: p.chosen.some((u) => u.status === 'locked'),
    uBulkLock: () => p.bulkLock(),
    uBulkUnlock: () => p.bulkUnlock(),
    uBulkRemove: () => p.bulkRemove(),
    uCanBulkAccess: p.paired.length > 0,
    uBulkAccessOpen: !!p.bulkAccess,
    uBulkAccessToggle: () => p.openBulkAccess(),
    uBulkAccess: nSel > 0 ? bulkAccessValues(p) : null,
    uHasRows: p.list.length > 0,
    uAllCk: {
      on: all,
      label: all === true ? m.users_sel_clear() : m.users_select_all(),
      icon: all === 'mixed' ? 'ti-minus' : 'ti-check',
      vis: p.pickable.length ? 1 : 0.35,
      onClick: () => p.toggleAll(),
    },
    uHdName: sortHead(p, 'name', m.users_head_name()),
    uHdRole: sortHead(p, 'role', m.users_head_role()),
    uHdStatus: sortHead(p, 'status', m.users_head_status()),
    uRows: rows,
    uEmpty: p.list.length === 0,
    uEmptyText: q ? m.users_empty_query({ q }) : m.users_empty(),
    uCanReset: !!q || p.filter !== 'all',
    uReset: () => p.resetFilter(),
  };
}
