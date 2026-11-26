/**
 * Sign-out of the hub's own pages (docs/app-contract.md › Signing out).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/logout.ts`) without the
 * account menu and the search.
 */
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { hideTooltip } from '#lib/actions/tooltip.ts';
import { m } from '#lib/paraglide/messages.js';
import { signOut } from '#lib/state/account.ts';
import { ui } from '#lib/state/ui.svelte.ts';

/** Sends pending settings, ends the session, discards the account's state and opens the sign-in. */
export async function logout(): Promise<void> {
  hideTooltip();
  // If it fails on the network, the hub stays signed in and shows the error.
  if (!(await signOut())) {
    ui.toasts.error(m.error_logout());
    return;
  }
  await goto(resolve('/login'), { replace: true });
}
