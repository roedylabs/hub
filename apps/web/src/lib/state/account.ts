/**
 * Start and end of a session in the browser: load the account after sign-in
 * or accepting an invitation, sign out, and a password that the link from the
 * mail has reset (then the server has already ended all sessions).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/state/account.ts`) without
 * the state of mail accounts, push and notifications.
 */
import { api, ApiError, ui } from '@roedy/ui';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { guest } from '#lib/state/guest.svelte.ts';
import { session } from '#lib/state/session.svelte.ts';

/**
 * After a new session (sign-in, invitation accepted, setup): first the settings (the account's theme), then the
 * account. `false` if the account is not signed in afterwards (cookie rejected, session ended in the meantime).
 */
export async function loadSignedIn(): Promise<boolean> {
  try {
    await prefs.load();
  } catch {
    // Without settings the defaults apply.
  }
  try {
    await session.refreshUser();
  } catch {
    return false;
  }
  if (session.authed) session.announce();
  return session.authed;
}

// The session ended without signing out in this tab, or another tab signed in a different account.
session.onEnd((next) => {
  // The error message that revealed the end (e.g. 401 on save) stays on the sign-in page.
  const failure = ui.toast?.error ? ui.toast.text : null;
  forgetAccount();
  if (failure) ui.toasts.error(failure);
  // A different account: its settings as after sign-in.
  if (next) void loadSignedIn();
});

/** Discards the account's state: toasts, settings and the account. */
export function forgetAccount(): void {
  ui.reset();
  guest.reset();
  prefs.reset();
  session.forget();
  // Other tabs of the same account ask the server and discard their state as well.
  session.announce();
}

/**
 * Sends pending settings, signs out at the server (docs/app-contract.md › Signing out) and discards the state.
 * `false` if signing out failed (then everything stays as it is); 401 means "already signed out".
 */
export async function signOut(): Promise<boolean> {
  await prefs.settled();
  try {
    await api('POST', 'auth/logout');
  } catch (error) {
    if (!(error instanceof ApiError && error.status === 401)) return false;
  }
  forgetAccount();
  return true;
}
