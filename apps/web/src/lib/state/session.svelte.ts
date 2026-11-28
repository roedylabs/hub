/**
 * State module `session`: the public configuration (`GET public/config`:
 * name, organization, branding, setup) and who is signed in
 * (`GET session`, docs/app-contract.md › Browser API of the hub). Loads both
 * at start; 401 from `session` means "not signed in".
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/state/session.svelte.ts`)
 * without the banner, maintenance mode, update hint, start copy, mail stores
 * and the polling of the configuration; the account comes from `GET session`
 * instead of `GET auth/me`.
 */
import { api, ApiError, BRAND0, mkLogo, type ApiOptions, type Branding, type HubPrefs } from '@roedy/ui';
import { legalLinksOf } from '#lib/links.ts';
import { m } from '#lib/paraglide/messages.js';

/** Organization (`instance::Org`); `null` means "not specified". */
export interface Org {
  name: string | null;
  website: string | null;
  help: string | null;
  email: string | null;
  imprint: string | null;
  privacy: string | null;
  terms: string | null;
  source: string | null;
}

export interface PublicConfig {
  /** The hub's name: the organization's, otherwise "Roedy Hub". */
  name: string;
  branding: Branding;
  org: Org;
  setup_required: boolean;
}

/** Response of `GET session` (docs/app-contract.md › `GET /api/v1/session`). */
export interface SessionInfo {
  user: {
    id: string;
    username: string;
    email: string;
    name: string;
    role: 'admin' | 'user';
    locale: 'en' | 'de';
  };
  /** The shared settings for the apps; the hub's own pages use `GET settings` (`prefs`). */
  prefs: HubPrefs;
  session: { id: string; auth_time: number };
  hub: { name: string; home: string; account: string; admin: string | null };
}

/** The signed-in account as the pages use it. */
export type Me = SessionInfo['user'];

/** Initials: first and last word. */
export function initialsOf(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const first = words[0] ?? '?';
  const last = words.length > 1 ? (words.at(-1) ?? '') : '';
  return (first.charAt(0) + last.charAt(0)).toUpperCase();
}

/** `BroadcastChannel` between the tabs: sign-in and sign-out. */
export const TAB_CHANNEL = 'roedy.hub.session';

export type SessionStatus = 'idle' | 'loading' | 'ready' | 'error';

export class Session {
  config = $state<PublicConfig | null>(null);
  user = $state<Me | null>(null);
  /** Session ID and time of the last sign-in (seconds) of `GET session`. */
  current = $state<SessionInfo['session'] | null>(null);
  status = $state<SessionStatus>('idle');
  /** Loading failed (`status` `error`): "offline" without a server response, otherwise "500". */
  failure = $state<'offline' | '500' | null>(null);

  appName = $derived(this.config?.name || m.app_name());
  branding = $derived(this.config?.branding ?? BRAND0);
  logo = $derived(mkLogo(this.branding, this.appName));
  /** Privacy policy and imprint of the organization for the footers of sign-in, invitation and "Confirm email". */
  legal = $derived(legalLinksOf(this.config?.org));
  authed = $derived(this.user !== null);
  isAdmin = $derived(this.user?.role === 'admin');
  setupRequired = $derived(this.config?.setup_required ?? false);
  userName = $derived(this.user?.name ?? '');
  initials = $derived(initialsOf(this.userName));

  /** Counts `load()`: only the most recent one writes its result (an older one may finish later). */
  #loadSeq = 0;
  #meSeq = 0;
  #ended: ((next: Me | null) => void) | null = null;
  /** `onEnd` is running: then this tab announces nothing; the others find out themselves or already know. */
  #ending = false;
  /** Channel of `watchTabs`; when sending over it, this tab does not receive its own announcement. */
  #channel: BroadcastChannel | null = null;

  /**
   * Loads configuration and account. Without an administrator (`setup_required`) it does not query `session`. The
   * setup calls it again after finishing. `signal` ends the requests early; without an answer it counts as offline.
   */
  async load(signal?: AbortSignal): Promise<void> {
    // Reloading (after the setup or "Try again") leaves the page in place until the response is there.
    if (this.status === 'idle') this.status = 'loading';
    const options: ApiOptions = signal ? { signal } : {};
    const load = ++this.#loadSeq;
    try {
      const config = await api<PublicConfig>('GET', 'public/config', undefined, options);
      if (load !== this.#loadSeq) return;
      const seq = ++this.#meSeq;
      const info = config.setup_required ? null : await this.#fetch(options);
      if (load !== this.#loadSeq) return;
      // Both at once, otherwise the layout would redirect in between after half the state.
      this.config = config;
      if (seq === this.#meSeq) this.#settle(info);
      this.status = 'ready';
      this.failure = null;
    } catch (error) {
      // A newer load has started in the meantime: its result applies.
      if (load !== this.#loadSeq) return;
      this.failure = error instanceof ApiError ? '500' : 'offline';
      this.status = 'error';
    }
  }

  /** Reloads `session`, e.g. after sign-in. */
  async refreshUser(): Promise<void> {
    // Only the most recent query counts: an older response (e.g. "signed out" from before sign-in) would otherwise discard the new session.
    const seq = ++this.#meSeq;
    const info = await this.#fetch();
    if (seq !== this.#meSeq) return;
    this.#settle(info);
  }

  /** Signed out in this tab: a `session` request that is still running must not bring the account back. */
  forget(): void {
    this.#meSeq++;
    this.user = null;
    this.current = null;
  }

  /**
   * Callback when the server reports no account or a different one instead of the signed-in account: the session
   * ended without signing out in this tab (expiry, sign-out in another tab, ended by the administration) or another
   * tab signed in a different account. Runs before `next` applies.
   */
  onEnd(handler: (next: Me | null) => void): void {
    this.#ended = handler;
  }

  #settle(info: SessionInfo | null): void {
    const user = info?.user ?? null;
    this.current = info?.session ?? null;
    const before = this.user?.id;
    if (before === undefined || before === user?.id) {
      this.user = user;
      return;
    }
    this.#ending = true;
    try {
      this.#ended?.(user);
    } finally {
      // Even if discarding fails, the server's state applies.
      this.#ending = false;
      this.user = user;
    }
  }

  /** Tells the other tabs of this browser which account is signed in now. */
  announce(): void {
    if (this.#ending || typeof BroadcastChannel === 'undefined') return;
    const own = this.#channel;
    const channel = own ?? new BroadcastChannel(TAB_CHANNEL);
    channel.postMessage(this.user?.id ?? null);
    if (!own) channel.close();
  }

  /**
   * Listens to `announce` of the other tabs: if one names an account other than this tab's, it reloads `session`
   * (then `onEnd` takes effect). The content of the message decides nothing; the server stays authoritative.
   * Returns the cleanup.
   */
  watchTabs(): () => void {
    if (typeof BroadcastChannel === 'undefined') return () => {};
    const channel = new BroadcastChannel(TAB_CHANNEL);
    channel.onmessage = (event: MessageEvent<unknown>) => {
      if (this.status === 'ready' && this.user && event.data !== this.user.id) void this.refreshUser().catch(() => {});
    };
    this.#channel = channel;
    return () => {
      if (this.#channel === channel) this.#channel = null;
      channel.close();
    };
  }

  async #fetch(options: ApiOptions = {}): Promise<SessionInfo | null> {
    try {
      return await api<SessionInfo>('GET', 'session', undefined, options);
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) return null;
      throw error;
    }
  }
}

export const session = new Session();
