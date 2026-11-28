/**
 * Return from a Stalwart server's sign-in page (`/oauth/callback`, the
 * redirect URI of the hub's OAuth client): the page takes `state` and `code`
 * (or the server's `error`) and `iss` (RFC 9207) from the address, removes
 * them from the address and the history at once, and hands them to `POST
 * stalwart/sign-in/complete`, where the hub exchanges the code, reads the
 * account and stores it.
 *
 * Done: on to `return_to` (checked again with `safeNext`; a hub page inside
 * the web app with a toast, an app's path with a full page load). An error
 * stays on the page with its text (`#lib/account-errors.ts`), "Back" to
 * `return_to` (the answer names it, else the attempt's, else Account ›
 * Accounts) and – where the attempt is known (`#lib/stalwart-sign-in.ts`) –
 * "Try again", which starts the same sign-in anew; `dup` offers "Open
 * account" instead.
 *
 * Roedy Mail's page (`apps/web/src/routes/oauth/callback/+page.svelte`) only
 * handed the return to its own sign-in flow; here the hub completes it.
 */
import { api, ApiError, isApiError, ui } from '@roedy/ui';
import { accountError, type AccountError } from '#lib/account-errors.ts';
import { safeNext } from '#lib/next.ts';
import { m } from '#lib/paraglide/messages.js';
import type { SignInComplete } from '#lib/stalwart.ts';
import { forgetAttempt, lastAttempt, startSignIn, type SignInAttempt } from '#lib/stalwart-sign-in.ts';

/** Where the return leads without another target. */
export const ACCOUNTS_PATH = '/account/accounts';

export interface CallbackNav {
  /** Removes code and `state` from the address (replaces the history entry). */
  clean: () => void;
  /** Opens a checked path in place of the page (`leaveTo`). */
  leave: (to: string) => void;
}

/** Codes after which "Try again" (a new sign-in with the same attempt) can help. */
const RETRY = new Set([
  'invalid_state',
  'session_changed',
  'sign_in_expired',
  'sign_in_failed',
  'issuer_mismatch',
  'server_changed',
  'exchange_failed',
  'other_account',
  'check_failed',
]);

/** Temporary failures, also of starting the sign-in again: "Try again" later can help. */
const TEMPORARY = new Set(['offline', 'rate_limited', 'sign_in_unavailable']);

/** The parameters of the return, as the server sent them. */
export interface Returned {
  state: string;
  code: string | null;
  error: string | null;
  iss: string | null;
}

/** Reads the return's parameters; `null` without `state`. */
export function returnedOf(url: URL): Returned | null {
  const params = url.searchParams;
  const state = params.get('state');
  if (!state) return null;
  return { state, code: params.get('code'), error: params.get('error'), iss: params.get('iss') };
}

export class CallbackFlow {
  readonly nav: CallbackNav;
  /** `busy` while the hub completes the sign-in; `error` with `error`. */
  phase = $state<'busy' | 'error'>('busy');
  error = $state<AccountError | null>(null);
  /** The error's code (`retry` decides by it). */
  code = $state('');
  /** "Try again" is starting the sign-in anew. */
  retrying = $state(false);
  /** Where "Back" leads. */
  returnTo = $state(ACCOUNTS_PATH);
  readonly attempt: SignInAttempt | null;
  #disposed = false;

  constructor(nav: CallbackNav, attempt: SignInAttempt | null = lastAttempt()) {
    this.nav = nav;
    this.attempt = attempt;
    this.returnTo = safeNext(attempt?.return_to) ?? ACCOUNTS_PATH;
  }

  /** Host the texts name (from the attempt; empty: "the server"). */
  get host(): string {
    return this.attempt?.host ?? '';
  }

  /** "Try again" is offered. */
  get canRetry(): boolean {
    return !!this.attempt && (RETRY.has(this.code) || TEMPORARY.has(this.code));
  }

  /** The page is gone: an answer that arrives later neither shows a toast nor navigates. */
  dispose(): void {
    this.#disposed = true;
  }

  async run(url: URL): Promise<void> {
    const returned = returnedOf(url);
    this.nav.clean();
    if (!returned) {
      this.#fail(new ApiError(400, { error: 'invalid_state' }));
      return;
    }
    const body: Record<string, string> = { state: returned.state };
    if (returned.error !== null) body.error = returned.error;
    else {
      body.code = returned.code ?? '';
      if (returned.iss !== null) body.iss = returned.iss;
    }
    try {
      const done = await api<SignInComplete>('POST', 'stalwart/sign-in/complete', body);
      forgetAttempt();
      if (this.#disposed) return;
      const to = safeNext(done.return_to) ?? ACCOUNTS_PATH;
      ui.toasts.show(
        this.attempt?.account_id
          ? m.acct_signed_in_again({ email: done.account.email })
          : m.acct_connected({ email: done.account.email }),
      );
      this.nav.leave(to);
    } catch (error) {
      this.#fail(error);
    }
  }

  #fail(error: unknown): void {
    if (this.#disposed) return;
    if (error instanceof ApiError) {
      const back = safeNext(typeof error.body.return_to === 'string' ? error.body.return_to : null);
      if (back) this.returnTo = back;
      this.code = error.code;
    } else this.code = 'offline';
    this.error = accountError(error, { host: this.host, email: this.attempt?.email ?? '' });
    this.phase = 'error';
  }

  /** "Back": to where the sign-in came from. */
  back(): void {
    this.nav.leave(this.returnTo);
  }

  /** "Open account" after `dup`. */
  openAccount(): void {
    const id = this.error?.account;
    this.nav.leave(id ? `${ACCOUNTS_PATH}/${id}` : ACCOUNTS_PATH);
  }

  /** "Try again": the same sign-in anew; the browser leaves for the server's sign-in page. */
  async retry(): Promise<void> {
    const attempt = this.attempt;
    if (!attempt || this.retrying) return;
    this.retrying = true;
    try {
      await startSignIn(attempt, () => !this.#disposed);
    } catch (error) {
      if (this.#disposed) return;
      this.retrying = false;
      this.code = error instanceof ApiError ? error.code : 'offline';
      this.error = accountError(error, { host: this.host, email: attempt.email ?? '' });
      // Errors of the start that "Try again" cannot cure.
      if (isApiError(error, 422) || isApiError(error, 409)) this.code = '';
    }
  }
}
