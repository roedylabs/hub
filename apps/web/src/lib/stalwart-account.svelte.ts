/**
 * Account › Accounts › one account (`/account/accounts/[id]`): state with
 * "Sign in again" or "Check", storage, colour, default account, the facts
 * of the connection and "Remove account".
 *
 * - Colour and default account: `PATCH stalwart/accounts/:id`, shown at once
 *   and put back with the error toast if saving fails.
 * - "Check": `POST …/check` reads the session and the storage with a token
 *   from the broker; a failure shows in the state card, the account stays.
 * - "Sign in again": an account signed in at the server starts the sign-in
 *   with `account_id` (`#lib/stalwart-sign-in.ts`) and returns here; an
 *   account with an access token gets a field for a new one (`POST
 *   …/reauth`). An account whose server the registry no longer has (or
 *   that lost its OAuth client) says so instead.
 * - "Remove": after the confirmation `DELETE`; the hub revokes the grant at
 *   the server. The hub's pages act at once (no undo).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/mail-account.svelte.ts` and
 * `lib/mail-connection.svelte.ts`) with the hub's API, without Mail's
 * per-account pages (identities, vacation reply, rules).
 */
import { api, failText, isApiError, loadFailText, Loading, type StalwartAccount, ui } from '@roedy/ui';
import { accountError, type AccountError } from '#lib/account-errors.ts';
import { Countdown } from '#lib/countdown.svelte.ts';
import { m } from '#lib/paraglide/messages.js';
import { session } from '#lib/state/session.svelte.ts';
import type { PublicServer, PublicServers } from '#lib/stalwart.ts';
import { startSignIn } from '#lib/stalwart-sign-in.ts';
import { hostOf } from '#lib/url.ts';

export type DetailStatus = 'loading' | 'ready' | 'missing' | 'failed';

/** Where the page leads. */
export interface AccountNav {
  /** Account › Accounts. */
  list: () => void;
  /** Administration › Stalwart servers. */
  servers: () => void;
}

/** Form "Sign in again" with an access token. */
export interface ReauthForm {
  token: string;
  showToken: boolean;
  busy: boolean;
  error: AccountError | null;
}

/** Capabilities of the session as the apps that use them. */
export const CAPABILITIES = [
  ['urn:ietf:params:jmap:mail', 'mail'],
  ['urn:ietf:params:jmap:calendars', 'calendars'],
  ['urn:ietf:params:jmap:contacts', 'contacts'],
] as const;

export type CapabilityKind = (typeof CAPABILITIES)[number][1];

export class AccountDetailPage {
  readonly id: string;
  readonly nav: AccountNav;
  account = $state<StalwartAccount | null>(null);
  servers = $state<PublicServer[]>([]);
  status = $state<DetailStatus>('loading');
  failure = $state('');
  readonly loading = new Loading();
  /** "Check" running. */
  checking = $state(false);
  /** Turns of the refresh icon (two per check). */
  turns = $state(0);
  /** Result of the last check on this page: `ok`, or what failed. */
  checked = $state<'ok' | AccountError | null>(null);
  /** "Sign in again" with an access token, open. */
  reauth = $state<ReauthForm | null>(null);
  /** The sign-in at the server is being prepared (`POST stalwart/sign-in`), or its error. */
  signIn = $state<{ busy: boolean; error: AccountError | null } | null>(null);
  readonly clock = new Countdown<'rate'>();
  #disposed = false;

  constructor(id: string, nav: AccountNav) {
    this.id = id;
    this.nav = nav;
  }

  async load(): Promise<void> {
    try {
      const [list, registry] = await this.loading.run(
        Promise.all([
          api<{ accounts: StalwartAccount[] }>('GET', 'stalwart/accounts'),
          api<PublicServers>('GET', 'stalwart/servers'),
        ]),
      );
      if (this.#disposed) return;
      this.servers = registry.servers;
      this.account = list.accounts.find((a) => a.id === this.id) ?? null;
      this.status = this.account ? 'ready' : 'missing';
    } catch (error) {
      if (this.#disposed) return;
      this.failure = loadFailText(error);
      this.status = 'failed';
    }
  }

  retry(): Promise<void> {
    this.status = 'loading';
    return this.load();
  }

  dispose(): void {
    this.#disposed = true;
    this.clock.stop();
  }

  /** Host of the account's server (`mail.example.com`). */
  get host(): string {
    return this.account ? hostOf(this.account.url) : '';
  }

  /** The registry server of the account, `null` for a custom one (or one removed from the registry). */
  get server(): PublicServer | null {
    const id = this.account?.server?.id;
    return (id && this.servers.find((s) => s.id === id)) || null;
  }

  /**
   * How the account signs in again: at the server (`signin`), with a new access token (`token`), or not at all
   * (`blocked`: a custom server while they are off; `noclient`: the server lost the hub's OAuth client or left the
   * registry).
   */
  get again(): 'signin' | 'token' | 'blocked' | 'noclient' {
    const a = this.account;
    if (!a || a.state === 'blocked') return 'blocked';
    if (a.auth === 'token') return 'token';
    return this.server?.sign_in ? 'signin' : 'noclient';
  }

  /** The apps' kinds of data the server offers to this account. */
  get capabilities(): CapabilityKind[] {
    const caps = this.account?.capabilities ?? [];
    return CAPABILITIES.filter(([urn]) => caps.includes(urn)).map(([, kind]) => kind);
  }

  #replace(account: StalwartAccount): void {
    this.account = account;
  }

  /** Colour of the account (index into `MA_COLORS`). */
  async setColor(color: number): Promise<void> {
    const before = this.account;
    if (!before || before.color === color) return;
    this.account = { ...before, color };
    try {
      const saved = await api<StalwartAccount>('PATCH', `stalwart/accounts/${this.id}`, { color });
      if (!this.#disposed && this.account?.color === color) this.#replace(saved);
    } catch (error) {
      if (this.#disposed) return;
      if (this.account?.color === color) this.account = { ...this.account, color: before.color };
      ui.toasts.error(failText(error, m.acct_save_failed()));
    }
  }

  /** "Make default". */
  async makeDefault(): Promise<void> {
    const before = this.account;
    if (!before || before.default) return;
    this.account = { ...before, default: true };
    try {
      const saved = await api<StalwartAccount>('PATCH', `stalwart/accounts/${this.id}`, { default: true });
      if (this.#disposed) return;
      this.#replace(saved);
      ui.toasts.show(m.acct_default_done({ email: saved.email }));
    } catch (error) {
      if (this.#disposed) return;
      if (this.account) this.account = { ...this.account, default: false };
      ui.toasts.error(failText(error, m.acct_save_failed()));
    }
  }

  /** "Check": session and storage at the server. One at a time. */
  async check(): Promise<void> {
    const a = this.account;
    if (!a || this.checking) return;
    this.checking = true;
    this.turns += 2;
    this.checked = null;
    try {
      const saved = await api<StalwartAccount>('POST', `stalwart/accounts/${this.id}/check`);
      if (this.#disposed) return;
      this.#replace(saved);
      this.checked = 'ok';
    } catch (error) {
      if (this.#disposed) return;
      if (isApiError(error, 404)) {
        this.status = 'missing';
        return;
      }
      // The broker found the account needs a new sign-in: the list says so too.
      if (isApiError(error, 409, 'needs_sign_in')) this.account = { ...a, state: 'needs_sign_in' };
      if (isApiError(error, 409, 'blocked')) this.account = { ...a, state: 'blocked' };
      this.checked = accountError(error, { host: this.host, email: a.email }, { token: a.auth === 'token' });
    } finally {
      if (!this.#disposed) this.checking = false;
    }
  }

  /** "Sign in again": at the server, or the access token field. */
  async startAgain(): Promise<void> {
    const a = this.account;
    if (!a) return;
    if (this.again === 'token') {
      this.reauth ??= { token: '', showToken: false, busy: false, error: null };
      return;
    }
    if (this.again !== 'signin' || this.signIn?.busy) return;
    this.signIn = { busy: true, error: null };
    try {
      await startSignIn({ account_id: a.id, return_to: `/account/accounts/${a.id}`, host: this.host });
    } catch (error) {
      if (this.#disposed) return;
      const failure = accountError(error, { host: this.host, email: a.email });
      if (failure.wait) this.clock.start('rate', failure.wait);
      this.signIn = { busy: false, error: failure };
    }
  }

  updateReauth(patch: Partial<ReauthForm>): void {
    if (!this.reauth || this.reauth.busy) return;
    this.reauth = { ...this.reauth, error: null, ...patch };
  }

  cancelReauth(): void {
    if (!this.reauth?.busy) this.reauth = null;
  }

  /** A new access token for an account connected with one. */
  async submitReauth(): Promise<void> {
    const form = this.reauth;
    const a = this.account;
    if (!form || !a || form.busy || this.clock.left('rate') > 0) return;
    const token = form.token.trim();
    if (!token) {
      this.reauth = {
        ...form,
        error: { icon: 'ti-key', title: m.acct_err_sec_title(), text: m.acct_err_sec_text(), field: 'token' },
      };
      return;
    }
    this.reauth = { ...form, busy: true, error: null };
    try {
      const saved = await api<StalwartAccount>('POST', `stalwart/accounts/${this.id}/reauth`, { token });
      if (this.#disposed) return;
      this.#replace(saved);
      this.reauth = null;
      this.checked = null;
      ui.toasts.show(m.acct_signed_in_again({ email: saved.email }));
    } catch (error) {
      if (this.#disposed || !this.reauth) return;
      const failure = accountError(error, { host: this.host, email: a.email }, { token: true });
      if (failure.wait) this.clock.start('rate', failure.wait);
      this.reauth = { ...this.reauth, busy: false, error: failure };
    }
  }

  /** "Remove account" after the confirmation. */
  remove(): void {
    const a = this.account;
    if (!a) return;
    ui.ask({
      title: m.acct_remove_title({ email: a.email }),
      text: m.acct_remove_text({ host: this.host, app: session.appName }),
      icon: 'ti-mail-off',
      ok: m.acct_remove(),
      danger: true,
      run: () => void this.#remove(a),
    });
  }

  async #remove(a: StalwartAccount): Promise<void> {
    try {
      await api('DELETE', `stalwart/accounts/${a.id}`);
    } catch (error) {
      // Already gone: as removed.
      if (!isApiError(error, 404)) {
        ui.toasts.error(failText(error, m.acct_remove_failed()));
        return;
      }
    }
    ui.toasts.show(m.acct_removed({ email: a.email }));
    if (!this.#disposed) this.nav.list();
  }
}
