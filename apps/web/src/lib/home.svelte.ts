/**
 * Home page against `GET apps` (docs/app-contract.md › Browser API): the
 * signed-in user's apps as tiles in the administrator's order, with the
 * offline state of each, and the empty state without any.
 *
 * Apps that wait for pairing are never among them, not even for
 * administrators; for them the page also loads `GET admin/apps` and points
 * to Administration › Apps while any app waits (and tells "no apps yet" from
 * "no access to the apps there are"). If that request fails, the hint and
 * the distinction are left out, the tiles stay.
 *
 * The list loads again when the tab becomes visible again (an app may have
 * come back online or access may have changed), quietly: a failure then
 * keeps the tiles. Only the latest request writes; nothing after `dispose`.
 *
 * The page has no design yet (docs/plan.md › Web app): it is built plainly
 * with the tokens.
 */
import { api, loadFailText, Loading } from '@roedy/ui';
import { appName, type AdminApp, type HomeApp } from '#lib/apps.ts';
import { normalizeName } from '#lib/name.ts';
import { m } from '#lib/paraglide/messages.js';
import { session } from '#lib/state/session.svelte.ts';

export type HomeStatus = 'loading' | 'ready' | 'failed';

export class HomePage {
  apps = $state<HomeApp[]>([]);
  status = $state<HomeStatus>('loading');
  /** Reason the first load failed (text of the card). */
  failure = $state('');
  readonly loading = new Loading();
  /** Administrators: names of the apps waiting for pairing (`admin/apps`); otherwise empty. */
  pending = $state<string[]>([]);
  /** Administrators: number of all apps (`admin/apps`); `null` for users or while unknown. */
  total = $state<number | null>(null);

  #seq = 0;
  #adminSeq = 0;
  #disposed = false;

  /** First load: the tiles and, for administrators, the apps of the administration. */
  start(): void {
    void this.load();
    if (session.isAdmin) void this.#loadAdmin();
  }

  /** Loads the tiles; after the first load a failure keeps them. */
  async load(): Promise<void> {
    const seq = ++this.#seq;
    try {
      const { apps } = await this.loading.run(api<{ apps: HomeApp[] }>('GET', 'apps'));
      if (seq !== this.#seq || this.#disposed) return;
      this.apps = apps;
      this.status = 'ready';
    } catch (error) {
      if (seq !== this.#seq || this.#disposed || this.status === 'ready') return;
      this.failure = loadFailText(error);
      this.status = 'failed';
    }
  }

  async #loadAdmin(): Promise<void> {
    const seq = ++this.#adminSeq;
    try {
      const { apps } = await api<{ apps: AdminApp[] }>('GET', 'admin/apps');
      if (seq !== this.#adminSeq || this.#disposed) return;
      this.pending = apps.filter((a) => a.state === 'pending').map(appName);
      this.total = apps.length;
    } catch {
      // Without it the page shows the tiles only.
    }
  }

  /** The tab became visible again: load quietly. */
  refresh(): void {
    if (this.#disposed || this.status === 'loading') return;
    void this.load();
    if (session.isAdmin) void this.#loadAdmin();
  }

  /** "Try again" of the card. */
  async retry(): Promise<void> {
    this.status = 'loading';
    await this.load();
  }

  dispose(): void {
    this.#disposed = true;
  }
}

/** Tile of an app. */
export interface Tile {
  slug: string;
  name: string;
  desc: string;
  url: string;
  icon: string;
  offline: boolean;
}

export interface HomeValues {
  /** "Hello, Alex". */
  greeting: string;
  lead: string;
  tiles: Tile[];
  empty: boolean;
  emptyTitle: string;
  emptyText: string;
  /** Administrators: to Administration › Apps from the empty state. */
  emptyAction: { label: string; onClick: () => void } | null;
  /** Administrators while apps wait for pairing. */
  pendingHint: { text: string; label: string; onClick: () => void } | null;
}

export interface HomeContext {
  /** Administration › Apps. */
  openApps: () => void;
}

export function homeValues(p: HomePage, ctx: HomeContext): HomeValues {
  const first = normalizeName(session.userName).split(' ')[0] ?? '';
  const admin = session.isAdmin;
  const empty = p.status === 'ready' && p.apps.length === 0;
  // Administrators: no app at all, or none they have access to.
  const noApps = admin && p.total === 0;
  const n = p.pending.length;
  return {
    greeting: first ? m.home_hello({ name: first }) : m.home_hello_anon(),
    lead: m.home_lead({ org: session.appName }),
    tiles: p.apps.map((a) => ({
      slug: a.slug,
      name: a.name,
      desc: a.description,
      url: a.url,
      icon: a.icon,
      offline: a.status === 'offline',
    })),
    empty,
    emptyTitle: noApps ? m.home_empty_admin_title() : m.home_empty_title(),
    emptyText: !admin ? m.home_empty_text() : noApps ? m.home_empty_admin_text() : m.home_empty_access_text(),
    emptyAction: admin && empty ? { label: m.home_open_apps(), onClick: ctx.openApps } : null,
    pendingHint:
      admin && n > 0
        ? {
            text: n === 1 ? m.home_pending_one({ name: p.pending[0] ?? '' }) : m.home_pending_many({ n }),
            label: m.home_pending_go(),
            onClick: ctx.openApps,
          }
        : null,
  };
}
