/**
 * "No account yet" (`/account/no-account?next=<path>`): where an app sends a
 * signed-in user without a Stalwart account (docs/app-contract.md › Links to
 * the hub). It names the app when `next` is an app's path (its name from
 * `GET apps`), says what the user needs and leads to "Add account" with the
 * same `next`, so that the user comes back to the app once the account is
 * there. A user who has accounts goes straight on to `next`.
 *
 * Failed requests do not block the page: without the accounts it still offers
 * "Add account", without the apps it speaks of "your apps".
 *
 * Taken over from Roedy Mail (`apps/web/src/routes/no-account/+page.svelte`)
 * as a page of the hub for every app.
 */
import { api, type StalwartAccount } from '@roedy/ui';
import type { HomeApp } from '#lib/apps.ts';
import { isHubPath } from '#lib/next.ts';

export interface NoAccountNav {
  /** Opens a checked path in place of the page (`leaveTo`). */
  leave: (to: string) => void;
}

/** Slug of an app's path (`/mail/inbox` → `mail`); `null` for the hub's own pages. */
export function slugOf(path: string): string | null {
  if (isHubPath(path)) return null;
  return path.split(/[/?#]/)[1] || null;
}

export class NoAccountPage {
  readonly next: string;
  readonly nav: NoAccountNav;
  /** Name of the app that sent the user; `null` if unknown. */
  app = $state<string | null>(null);
  /** The accounts were checked (the page shows only then, so that a user with accounts sees no flash). */
  ready = $state(false);
  #disposed = false;

  constructor(next: string, nav: NoAccountNav) {
    this.next = next;
    this.nav = nav;
  }

  async load(): Promise<void> {
    const slug = slugOf(this.next);
    const apps = slug ? api<{ apps: HomeApp[] }>('GET', 'apps').catch(() => null) : Promise.resolve(null);
    try {
      const { accounts } = await api<{ accounts: StalwartAccount[] }>('GET', 'stalwart/accounts');
      if (this.#disposed) return;
      if (accounts.length > 0) {
        this.nav.leave(this.next);
        return;
      }
    } catch {
      // Unknown: the page offers adding one.
    }
    const list = await apps;
    if (this.#disposed) return;
    this.app = list?.apps.find((a) => a.slug === slug)?.name ?? null;
    this.ready = true;
  }

  dispose(): void {
    this.#disposed = true;
  }

  /** Address of "Add account" with the same `next`. */
  get addHref(): string {
    return this.next === '/' ? '/account/accounts/new' : `/account/accounts/new?next=${encodeURIComponent(this.next)}`;
  }
}
