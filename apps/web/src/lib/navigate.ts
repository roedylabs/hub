/**
 * Leaving a page of the hub for a checked target (`#lib/next.ts`): the hub's
 * own pages open inside the web app, an app's through the gateway with a full
 * page load.
 */
import { goto } from '$app/navigation';
import { isHubPath } from '#lib/next.ts';

/** Opens `to` in place of the current page (no history entry for the page left). */
export function leaveTo(to: string): void {
  // eslint-disable-next-line svelte/no-navigation-without-resolve -- `to` is a path checked by `safeNext`
  if (isHubPath(to)) void goto(to, { replace: true });
  else location.replace(to);
}
