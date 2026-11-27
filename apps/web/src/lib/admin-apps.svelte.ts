/**
 * Administration › Apps against `admin/apps` (`api::apps` of the server):
 * the list with status and order (`AppsPage`) and the page of one app
 * (`AppPage`).
 *
 * List: the apps in their order on the home page with status and number of
 * users. "Move up"/"Move down" (buttons, so the order works with the
 * keyboard and a screen reader, which hears the new position) change the
 * list at once and send the whole order (`PATCH admin/apps`, every slug
 * once); the requests go out one after another (`serial`). A 422 means an
 * app was added or removed in the meantime: the list loads again with a
 * toast; other failures load it again too. "Add app" asks for the slug (the
 * address `/<slug>/`) and the internal address; the response carries the
 * first pairing code, which is shown once with "Copy" until it is closed.
 * Field errors are checked before sending like the server does
 * (`apps::check_slug`, `apps::internal_origin`) and shown at the field, as
 * are the server's (`fields.slug`, `fields.internal_url`).
 *
 * App page: status, addresses, version, the event delivery, "New pairing
 * code" (shown once), the access default for new users (`PATCH
 * admin/apps/:slug`; existing access stays as it is, the page says so), the
 * roles of the manifest, who has access (`GET admin/apps/:slug/users`) and
 * "Remove app" after a confirmation. A 404 shows "App not found".
 *
 * Only the latest load writes; nothing writes after `dispose`, failed actions
 * still show their toast. 401/403 reload the account (`lostAccess`).
 */
import { lostAccess } from '#lib/access.ts';
import { api, fieldsOf, isApiError } from '#lib/api.ts';
import { appName, type AccessDefault, type AdminApp, type Pairing } from '#lib/apps.ts';
import { copyText } from '#lib/clipboard.svelte.ts';
import { failText, loadFailText } from '#lib/errors.ts';
import { m } from '#lib/paraglide/messages.js';
import { serial } from '#lib/serial.ts';
import { Loading, ui } from '#lib/state/ui.svelte.ts';
import { trimWs } from '#lib/trim.ts';

export type LoadStatus = 'loading' | 'ready' | 'failed';

/** Reasons of `fields.slug`. */
export type SlugError = 'required' | 'invalid' | 'reserved' | 'taken';
/** Reasons of `fields.internal_url`. */
export type UrlError = 'required' | 'invalid';

/** First path segments the hub keeps (`RESERVED_SLUGS` of the contract crate). */
export const RESERVED_SLUGS: readonly string[] = [
  'login',
  'account',
  'admin',
  'setup',
  'invite',
  'api',
  'internal',
  'healthz',
  'readyz',
  '.well-known',
  '_hub',
  'favicon.ico',
  'robots.txt',
  'apps',
  'auth',
  'branding',
  'error',
  'help',
  'hub',
  'logout',
  'oauth',
  'settings',
  'static',
];

/** Slug error like `apps::check_slug` (2–32 of `a-z0-9-`, a letter first, no hyphen last, not reserved). */
export function slugError(slug: string): SlugError | null {
  const s = slug.trim();
  if (!s) return 'required';
  if (RESERVED_SLUGS.includes(s)) return 'reserved';
  return /^[a-z][a-z0-9-]{0,30}[a-z0-9]$/.test(s) ? null : 'invalid';
}

/** Internal address like `apps::internal_origin`: `http(s)://host[:port]`, nothing else. */
export function internalUrlError(value: string): UrlError | null {
  const t = value.trim();
  if (!t) return 'required';
  try {
    // eslint-disable-next-line svelte/prefer-svelte-reactivity -- parsed once to check it, not state
    const url = new URL(t);
    const ok =
      (url.protocol === 'http:' || url.protocol === 'https:') &&
      url.hostname !== '' &&
      !url.username &&
      !url.password &&
      url.pathname === '/' &&
      !url.search &&
      !url.hash &&
      // `URL` drops an empty query or fragment: they must not be there either.
      !/[?#]/.test(t);
    return ok ? null : 'invalid';
  } catch {
    return 'invalid';
  }
}

export function slugErrorText(code: SlugError): string {
  switch (code) {
    case 'required':
      return m.apps_err_slug_required();
    case 'reserved':
      return m.apps_err_slug_reserved();
    case 'taken':
      return m.apps_err_slug_taken();
    default:
      return m.apps_err_slug_invalid();
  }
}

export function urlErrorText(code: UrlError): string {
  return code === 'required' ? m.apps_err_url_required() : m.apps_err_url_invalid();
}

/** Copies a pairing code; the toast says whether it worked. */
async function copyCode(code: string): Promise<void> {
  if (await copyText(code)) ui.toasts.show(m.apps_code_copied());
  else ui.toasts.error(m.error_copy());
}

/** Form "Add app". */
interface AddForm {
  /** Counts up per opened form: responses only apply to their form. */
  id: number;
  slug: string;
  internal: string;
  busy: boolean;
  /** Errors at the fields: after "Add" or from the server, until the field changes. */
  slugErr: SlugError | null;
  urlErr: UrlError | null;
}

/** The pairing code just made, for the app it belongs to. */
export interface ShownCode {
  slug: string;
  name: string;
  pairing: Pairing;
}

export class AppsPage {
  rows = $state<AdminApp[]>([]);
  status = $state<LoadStatus>('loading');
  failure = $state('');
  readonly loading = new Loading();
  add = $state<AddForm | null>(null);
  /** Pairing code of the app just added. */
  shown = $state<ShownCode | null>(null);
  /** Announcement after moving an app (live region). */
  moved = $state('');

  #seq = 0;
  #formId = 0;
  #disposed = false;
  readonly #order = serial();
  /** Order writes not settled yet. */
  #ordering = 0;
  /** An order write failed: load the list once all have settled. */
  #reconcile = false;

  start(): void {
    void this.load();
  }

  async load(): Promise<void> {
    const seq = ++this.#seq;
    try {
      const { apps } = await this.loading.run(api<{ apps: AdminApp[] }>('GET', 'admin/apps'));
      if (seq !== this.#seq || this.#disposed) return;
      this.rows = apps;
      this.status = 'ready';
    } catch (error) {
      if (seq !== this.#seq || this.#disposed) return;
      lostAccess(error);
      if (this.status !== 'ready') {
        this.failure = loadFailText(error);
        this.status = 'failed';
      } else ui.toasts.error(failText(error, m.apps_list_failed()));
    }
  }

  async retry(): Promise<void> {
    this.status = 'loading';
    await this.load();
  }

  dispose(): void {
    this.#disposed = true;
  }

  // Order

  /** Moves an app by `delta` places (−1 up, +1 down) and sends the new order. */
  move(slug: string, delta: -1 | 1): void {
    const from = this.rows.findIndex((a) => a.slug === slug);
    const to = from + delta;
    if (from < 0 || to < 0 || to >= this.rows.length) return;
    const rows = [...this.rows];
    const [app] = rows.splice(from, 1);
    if (!app) return;
    rows.splice(to, 0, app);
    this.rows = rows;
    this.moved = m.apps_moved({ name: appName(app), n: to + 1, total: rows.length });
    const order = rows.map((a) => a.slug);
    // Invalidate a list load on its way: it may predate this order.
    this.#seq++;
    this.#ordering++;
    void this.#order(() => api<{ apps: AdminApp[] }>('PATCH', 'admin/apps', { order }))
      .then(
        () => {},
        (error: unknown) => {
          if (isApiError(error, 422)) ui.toasts.error(m.apps_order_changed());
          else ui.toasts.error(failText(error, m.apps_order_failed()));
          if (!lostAccess(error)) this.#reconcile = true;
        },
      )
      .finally(() => {
        // Reload only once the queued writes have settled, so the list shows the order the server ended with;
        // a move after that invalidates this load (`#seq`).
        if (--this.#ordering > 0 || !this.#reconcile) return;
        this.#reconcile = false;
        if (!this.#disposed) void this.load();
      });
  }

  // Adding

  openAdd(): void {
    this.add = { id: ++this.#formId, slug: '', internal: '', busy: false, slugErr: null, urlErr: null };
    this.shown = null;
  }

  closeAdd(): void {
    this.add = null;
  }

  #setForm(patch: Partial<AddForm>): void {
    if (this.add) this.add = { ...this.add, ...patch };
  }

  setSlug(value: string): void {
    // Slugs are lower case; typing upper case is no error.
    this.#setForm({ slug: value.toLowerCase(), slugErr: null });
  }

  setInternal(value: string): void {
    this.#setForm({ internal: value, urlErr: null });
  }

  async submitAdd(): Promise<void> {
    const form = this.add;
    if (!form || form.busy) return;
    const slug = trimWs(form.slug);
    const internal = trimWs(form.internal);
    const slugErr = slugError(slug);
    const urlErr = internalUrlError(internal);
    if (slugErr || urlErr) {
      this.#setForm({ slugErr, urlErr });
      return;
    }
    this.#setForm({ busy: true });
    const current = () => this.add?.id === form.id;
    try {
      const res = await api<{ app: AdminApp; pairing: Pairing }>('POST', 'admin/apps', {
        slug,
        internal_url: internal,
      });
      if (this.#disposed) return;
      if (current()) this.add = null;
      this.shown = { slug: res.app.slug, name: appName(res.app), pairing: res.pairing };
      ui.toasts.show(m.apps_added({ slug: res.app.slug }));
      if (!this.rows.some((a) => a.slug === res.app.slug)) this.rows = [...this.rows, res.app];
      void this.load();
    } catch (error) {
      if (this.#disposed || !current()) {
        ui.toasts.error(failText(error));
        return;
      }
      this.#setForm({ busy: false });
      if (isApiError(error, 422)) {
        const fields = fieldsOf(error);
        const s = fields.slug?.code as SlugError | undefined;
        const u = fields.internal_url?.code as UrlError | undefined;
        if (s || u) {
          this.#setForm({ slugErr: s ?? null, urlErr: u ?? null });
          return;
        }
      }
      if (!lostAccess(error)) ui.toasts.error(failText(error));
    }
  }

  closeShown(): void {
    this.shown = null;
  }

  copyShown(): void {
    if (this.shown) void copyCode(this.shown.pairing.code);
  }
}

/** Row of "Access" on an app's page (`GET admin/apps/:slug/users`). */
export interface AppUser {
  id: string;
  status: 'active' | 'locked' | 'invited';
  name: string | null;
  email: string;
  username?: string | null;
  role: 'admin' | 'user';
  app_role: string | null;
}

export type AppPageStatus = LoadStatus | 'gone';

export interface AppPageContext {
  /** After removing: back to the list. */
  leave: () => void;
}

export class AppPage {
  app = $state<AdminApp | null>(null);
  status = $state<AppPageStatus>('loading');
  failure = $state('');
  readonly loading = new Loading();
  users = $state<AppUser[]>([]);
  usersStatus = $state<LoadStatus>('loading');
  /** The pairing code just made (shown once). */
  pairing = $state<Pairing | null>(null);
  /** "New pairing code" or the access default on its way. */
  busy = $state(false);

  readonly #slug: string;
  readonly #ctx: AppPageContext;
  #seq = 0;
  #usersSeq = 0;
  #disposed = false;
  readonly #save = serial();
  /** Counts `setDefault` calls: only the latest one's response or failure shows. */
  #defaultGen = 0;
  /** Default saves not settled yet; a reload waits for them (`#reload`). */
  #saving = 0;
  #reloadAfterSaves = false;

  constructor(slug: string, ctx: AppPageContext) {
    this.#slug = slug;
    this.#ctx = ctx;
  }

  get slug(): string {
    return this.#slug;
  }

  start(): void {
    void this.load();
  }

  async load(): Promise<void> {
    const seq = ++this.#seq;
    try {
      const { app } = await this.loading.run(api<{ app: AdminApp }>('GET', `admin/apps/${this.#slug}`));
      if (seq !== this.#seq || this.#disposed) return;
      this.app = app;
      this.status = 'ready';
      if (app.state === 'paired') void this.loadUsers();
    } catch (error) {
      if (seq !== this.#seq || this.#disposed) return;
      if (isApiError(error, 404)) {
        this.status = 'gone';
        return;
      }
      lostAccess(error);
      if (this.status !== 'ready') {
        this.failure = loadFailText(error);
        this.status = 'failed';
      } else ui.toasts.error(failText(error, m.apps_app_failed()));
    }
  }

  async loadUsers(): Promise<void> {
    const seq = ++this.#usersSeq;
    try {
      const res = await api<{ users: AppUser[]; invites: AppUser[] }>('GET', `admin/apps/${this.#slug}/users`);
      if (seq !== this.#usersSeq || this.#disposed) return;
      this.users = [...res.users, ...res.invites];
      this.usersStatus = 'ready';
    } catch {
      if (seq !== this.#usersSeq || this.#disposed) return;
      if (this.usersStatus !== 'ready') this.usersStatus = 'failed';
    }
  }

  async retry(): Promise<void> {
    this.status = 'loading';
    await this.load();
  }

  dispose(): void {
    this.#disposed = true;
  }

  /** A failed action: toast; 404 shows "App not found", 401/403 reload the account, others reload the app. */
  #failed(error: unknown, text?: string): void {
    if (isApiError(error, 404)) {
      ui.toasts.error(m.apps_gone());
      if (!this.#disposed) this.status = 'gone';
      return;
    }
    ui.toasts.error(failText(error, text));
    if (this.#disposed || lostAccess(error)) return;
    this.#reload();
  }

  /** Loads the app again, once the queued default saves have settled: it then reads the default the server ended with. */
  #reload(): void {
    if (this.#saving > 0) this.#reloadAfterSaves = true;
    else void this.load();
  }

  async newCode(): Promise<void> {
    if (this.busy || !this.app) return;
    this.busy = true;
    try {
      const res = await api<{ app: AdminApp; pairing: Pairing }>('POST', `admin/apps/${this.#slug}/pairing-code`);
      if (this.#disposed) return;
      this.app = res.app;
      this.pairing = res.pairing;
    } catch (error) {
      this.#failed(error);
    } finally {
      this.busy = false;
    }
  }

  closeCode(): void {
    this.pairing = null;
  }

  copyCode(): void {
    if (this.pairing) void copyCode(this.pairing.code);
  }

  /** Access default for new users: shown at once, back on an error. */
  setDefault(value: AccessDefault): void {
    const app = this.app;
    if (!app || app.access_default === value) return;
    const before = app.access_default;
    const gen = ++this.#defaultGen;
    // A load on its way predates this change: it would show the old default.
    this.#seq++;
    this.app = { ...app, access_default: value };
    const current = () => !this.#disposed && gen === this.#defaultGen && !!this.app;
    this.#saving++;
    void this.#save(() => api<{ app: AdminApp }>('PATCH', `admin/apps/${this.#slug}`, { access_default: value }))
      .then(
        (res) => {
          if (current()) this.app = res.app;
          ui.toasts.show(m.apps_default_saved());
        },
        (error: unknown) => {
          if (current() && this.app) this.app = { ...this.app, access_default: before };
          // Every failure loads the app again (after the queued saves): the server's default then shows.
          this.#failed(error, m.apps_default_failed());
        },
      )
      .finally(() => {
        if (--this.#saving > 0 || !this.#reloadAfterSaves) return;
        this.#reloadAfterSaves = false;
        if (!this.#disposed) void this.load();
      });
  }

  /** "Remove app" after confirmation; then back to the list. */
  remove(): void {
    const app = this.app;
    if (!app) return;
    const name = appName(app);
    ui.ask({
      title: m.apps_remove_title({ name }),
      text: m.apps_remove_text({ name }),
      icon: 'ti-trash',
      ok: m.apps_remove(),
      danger: true,
      run: () => {
        void api('DELETE', `admin/apps/${this.#slug}`)
          .catch((error: unknown) => {
            // Removed in the meantime: done.
            if (!isApiError(error, 404)) throw error;
          })
          .then(
            () => {
              ui.toasts.show(m.apps_removed({ name }));
              if (!this.#disposed) this.#ctx.leave();
            },
            (error: unknown) => this.#failed(error, m.apps_remove_failed()),
          );
      },
    });
  }
}
