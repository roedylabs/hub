/**
 * The sign-in at a Stalwart server with the hub's OAuth client: `POST
 * stalwart/sign-in` keeps `state` and the PKCE verifier in the hub and answers
 * with the server's sign-in page, which the browser then opens. The server
 * sends the browser back to `/oauth/callback`, whose page completes the
 * sign-in (`#lib/callback.svelte.ts`).
 *
 * The attempt (server, address or account, where to return, the host for the
 * texts) stays in the tab's `sessionStorage`: the return page names the
 * server in its texts and offers "Try again" with it. It holds no secret.
 *
 * Unlike Roedy Mail (`apps/web/src/lib/auth/stalwart-login.ts`) the browser
 * creates neither `state` nor the verifier and never sees the code's tokens.
 */
import { api } from '@roedy/ui';
import type { SignInStart } from '#lib/stalwart.ts';

/** What a sign-in started with. */
export interface SignInAttempt {
  /** New account at this registry server with this address … */
  server_id?: string;
  email?: string;
  /** … or "Sign in again" for this account. */
  account_id?: string;
  /** Path on the hub's origin to return to. */
  return_to: string;
  /** Host of the server, for the texts of the return page. */
  host: string;
}

const KEY = 'roedy-hub.stalwart-sign-in';

/** Leaving for the server's sign-in page (replaced in tests). */
export const signInNav = {
  open: (url: string): void => location.assign(url),
};

/** The last attempt of this tab, or `null`. */
export function lastAttempt(): SignInAttempt | null {
  try {
    const raw = sessionStorage.getItem(KEY);
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<SignInAttempt>;
    if (typeof value.return_to !== 'string' || typeof value.host !== 'string') return null;
    return value as SignInAttempt;
  } catch {
    return null;
  }
}

/** Forgets the attempt (after it completed). */
export function forgetAttempt(): void {
  try {
    sessionStorage.removeItem(KEY);
  } catch {
    // Storage blocked: nothing kept.
  }
}

/**
 * Starts the sign-in and opens the server's sign-in page. Rejects with the API's error (field errors, `no_client`,
 * `blocked`, `sign_in_unavailable`, `rate_limited` …); then nothing is kept. `active` is asked once the answer is
 * there: a page that is gone by then (the user went elsewhere) neither keeps the attempt nor leaves for the server.
 */
export async function startSignIn(attempt: SignInAttempt, active: () => boolean = () => true): Promise<void> {
  const body: Record<string, string> = { return_to: attempt.return_to };
  if (attempt.account_id) body.account_id = attempt.account_id;
  else {
    body.server_id = attempt.server_id ?? '';
    body.email = attempt.email ?? '';
  }
  const { authorize_url } = await api<SignInStart>('POST', 'stalwart/sign-in', body);
  if (!active()) return;
  try {
    sessionStorage.setItem(KEY, JSON.stringify(attempt));
  } catch {
    // Without storage the return page has no host and no "Try again".
  }
  signInNav.open(authorize_url);
}
