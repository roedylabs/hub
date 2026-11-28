/**
 * Account › Accounts: the user's Stalwart accounts (`GET stalwart/accounts`)
 * and "Add account" against the registry (`GET stalwart/servers`).
 *
 * The form belongs to `/account/accounts/new` (`adding`); `?next=` (checked
 * with `safeNext`) is where it leads once the account is there – an app that
 * sent the user over "No account yet" gets them back. The server comes from
 * the address's domain ("Automatic"), from the list, or – while the
 * administrator allows custom servers – as an address of its own ("Other
 * server …"). A registry server with the hub's OAuth client signs in at the
 * server (`#lib/stalwart-sign-in.ts`, the browser leaves for its sign-in
 * page); otherwise, and on custom servers, the user enters an access token
 * (`POST stalwart/accounts`).
 *
 * Errors show in the form (`#lib/account-errors.ts`) at the field they belong
 * to; 429 locks "Continue" with a countdown.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/mail-accounts.svelte.ts` and
 * `lib/auth/stalwart-login.ts`) with the hub's API: the hub starts the sign-in
 * and exchanges the code, the address goes to the server as `login_hint`,
 * there is no "All accounts" and no password field (Mail's "Password" was
 * the sign-in at the server; here "Sign in at the server").
 */
import { api, isApiError, loadFailText, Loading, type StalwartAccount, ui } from '@roedy/ui';
import { accountError, type AccountError } from '#lib/account-errors.ts';
import { Countdown } from '#lib/countdown.svelte.ts';
import { isEmail } from '#lib/email.ts';
import { m } from '#lib/paraglide/messages.js';
import type { PublicServer, PublicServers } from '#lib/stalwart.ts';
import { startSignIn } from '#lib/stalwart-sign-in.ts';
import { hostOf, originOf } from '#lib/url.ts';

export type ListStatus = 'loading' | 'ready' | 'failed';

/** Choice of the server picker: the address's domain, a registry server's ID, or an address of its own. */
export type ServerChoice = 'auto' | 'other' | (string & {});

/** How the account connects: sign-in at the server (OAuth) or an access token. */
export type AddMethod = 'signin' | 'token';

/** The form "Add account". */
export interface AddForm {
  email: string;
  srv: ServerChoice;
  /** Address of "Other server …". */
  url: string;
  method: AddMethod;
  token: string;
  showToken: boolean;
  /** `open`: the sign-in page is being prepared; `token`: the access token is being checked. */
  busy: 'open' | 'token' | null;
  error: AccountError | null;
}

/** Where the page leads; the route sets them with `goto` and `resolve`. */
export interface AccountsNav {
  /** "Add account": `/account/accounts/new` (with `next`). */
  add: () => void;
  /** Form closed: `/account/accounts`. */
  list: () => void;
  /** An account's page. */
  account: (id: string) => void;
  /** Administration › Stalwart servers. */
  servers: () => void;
  /** After adding with `next`: there (an app's path with a full page load). */
  leave: (to: string) => void;
}

/** Where the account belongs. */
export interface Target {
  /** The registry server; `null` for a custom one. */
  server: PublicServer | null;
  /** Origin; empty for an invalid custom address. */
  url: string;
}

/** Domain of an address, lower case; empty without one. */
export function domainOf(email: string): string {
  return email.trim().toLowerCase().split('@')[1] ?? '';
}

/** Registry server for the address's domain; with a single server that one. */
export function serverForEmail(email: string, servers: readonly PublicServer[]): PublicServer | null {
  const domain = domainOf(email);
  const match = domain ? servers.find((s) => s.domains.includes(domain)) : undefined;
  if (match) return match;
  return servers.length === 1 ? (servers[0] ?? null) : null;
}

/** Origin of an address of its own, only with `https://`; empty if it is none. */
export function customOrigin(value: string): string {
  const url = value.trim();
  return /^https:\/\//i.test(url) ? originOf(url) : '';
}

/**
 * Server for the form's choice; `null` if none applies. An address of its own with a registry server's origin counts
 * as that server (as in the backend); otherwise only while custom servers are allowed.
 */
export function targetOf(
  form: Pick<AddForm, 'srv' | 'email' | 'url'>,
  servers: readonly PublicServer[],
  allowCustom: boolean,
): Target | null {
  if (form.srv === 'other') {
    const url = customOrigin(form.url);
    const server = (url && servers.find((s) => s.url === url)) || null;
    return server || allowCustom ? { url, server } : null;
  }
  const server = form.srv === 'auto' ? serverForEmail(form.email, servers) : servers.find((s) => s.id === form.srv);
  return server ? { url: server.url, server } : null;
}

const emptyForm = (): AddForm => ({
  email: '',
  srv: 'auto',
  url: '',
  method: 'signin',
  token: '',
  showToken: false,
  busy: null,
  error: null,
});

export class AccountsPage {
  readonly nav: AccountsNav;
  readonly adding: boolean;
  /** Checked `next` of `/account/accounts/new`; `null` without one. */
  readonly next: string | null;
  accounts = $state<StalwartAccount[]>([]);
  servers = $state<PublicServer[]>([]);
  allowCustom = $state(false);
  status = $state<ListStatus>('loading');
  /** Reason the first load failed (text of the card). */
  failure = $state('');
  readonly loading = new Loading();
  /** Open form (`/account/accounts/new`). */
  form = $state<AddForm | null>(null);
  /** Highlighted row of the server picker. */
  srvHi = $state<number | null>(null);
  /** "Try again in …" after 429. */
  readonly clock = new Countdown<'rate'>();
  #disposed = false;

  constructor(nav: AccountsNav, adding: boolean, next: string | null = null) {
    this.nav = nav;
    this.adding = adding;
    this.next = next;
    if (adding) this.form = emptyForm();
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
      this.accounts = list.accounts;
      this.servers = registry.servers;
      this.allowCustom = registry.allow_custom_servers;
      this.status = 'ready';
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

  /** "Add account": from the list to the form; on the form's page (closed after an error) open it again. */
  add(): void {
    if (this.adding) this.form ??= emptyForm();
    else this.nav.add();
  }

  /** "Cancel" and Escape: back to the list (an app that sent the user here gets them back without an account). */
  cancel(): void {
    if (this.form?.busy) return;
    this.form = null;
    if (this.next) this.nav.leave(this.next);
    else this.nav.list();
  }

  update(patch: Partial<AddForm>): void {
    if (!this.form || this.form.busy) return;
    const next = { ...this.form, ...patch };
    // A change of what an error names takes it away.
    if (this.form.error && !('error' in patch)) next.error = null;
    this.form = next;
  }

  pickServer(srv: ServerChoice): void {
    this.update({ srv });
    this.srvHi = null;
    if (ui.menu === 'pick') ui.closeMenu();
  }

  /** Server of the form's choice. */
  get target(): Target | null {
    return this.form ? targetOf(this.form, this.servers, this.allowCustom) : null;
  }

  /**
   * The chosen server signs in with the hub's OAuth client; before one is chosen (no address yet), whether any
   * registry server does, so that the form does not switch back and forth while typing.
   */
  get canSignIn(): boolean {
    const t = this.target;
    if (t) return !!t.server?.sign_in;
    return this.form?.srv !== 'other' && this.servers.some((s) => s.sign_in);
  }

  /** The method that applies: the sign-in only where the server has the hub's client. */
  get method(): AddMethod {
    return this.canSignIn && this.form?.method !== 'token' ? 'signin' : 'token';
  }

  /** Host of the chosen server for the texts; empty without one. */
  get host(): string {
    const t = this.target;
    return t?.server?.host ?? (t?.url ? hostOf(t.url) : '');
  }

  /** Remaining lock in whole seconds; 0 = free. */
  get lockLeft(): number {
    return this.clock.left('rate');
  }

  /** Error of the form before anything is sent; `null` if it can go. */
  check(): AccountError | null {
    const form = this.form;
    if (!form) return null;
    if (!isEmail(form.email.trim()))
      return { icon: 'ti-at', title: m.acct_err_email_title(), text: m.acct_err_email_text(), field: 'email' };
    if (form.srv === 'other' && !customOrigin(form.url))
      return { icon: 'ti-world', title: m.acct_err_url_title(), text: m.acct_err_url_text(), field: 'url' };
    if (!this.target) {
      if (form.srv === 'other')
        return {
          icon: 'ti-server-off',
          title: m.acct_err_custom_title(),
          text: m.acct_err_custom_text(),
          field: 'url',
        };
      return {
        icon: 'ti-server',
        title: m.acct_err_nosrv_title(),
        text: m.acct_err_nosrv_text({ domain: domainOf(form.email) || m.acct_this_domain() }),
        field: 'server',
      };
    }
    if (this.method === 'token' && !form.token.trim())
      return { icon: 'ti-key', title: m.acct_err_sec_title(), text: m.acct_err_sec_text(), field: 'token' };
    return null;
  }

  /** "Continue to sign-in" or "Connect". */
  async submit(): Promise<void> {
    const form = this.form;
    if (!form || form.busy || this.lockLeft > 0) return;
    const problem = this.check();
    if (problem) {
      this.form = { ...form, error: problem };
      return;
    }
    const target = this.target;
    if (!target) return;
    const email = form.email.trim().toLowerCase();
    const c = { host: this.host, email };
    if (this.method === 'signin' && target.server) {
      this.form = { ...form, busy: 'open', error: null };
      try {
        await startSignIn(
          {
            server_id: target.server.id,
            email,
            return_to: this.next ?? '/account/accounts',
            host: this.host,
          },
          () => !this.#disposed,
        );
        // The browser leaves for the sign-in page; the form stays busy until then.
      } catch (error) {
        this.#failed(error, c, false);
      }
      return;
    }
    this.form = { ...form, busy: 'token', error: null };
    try {
      const body = {
        ...(target.server ? { server_id: target.server.id } : { url: target.url }),
        email,
        token: form.token.trim(),
      };
      const account = await api<StalwartAccount>('POST', 'stalwart/accounts', body);
      if (this.#disposed) return;
      this.accounts = [...this.accounts.filter((a) => a.id !== account.id), account];
      this.form = null;
      ui.toasts.show(m.acct_connected({ email: account.email }));
      if (this.next) this.nav.leave(this.next);
      else this.nav.account(account.id);
    } catch (error) {
      this.#failed(error, c, true);
    }
  }

  #failed(error: unknown, c: { host: string; email: string }, token: boolean): void {
    if (this.#disposed || !this.form) return;
    const failure = accountError(error, c, { token });
    if (isApiError(error, 429) && failure.wait) this.clock.start('rate', failure.wait);
    this.form = { ...this.form, busy: null, error: failure };
  }
}
