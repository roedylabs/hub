/**
 * Lost session or role on an administration page: a 401 or 403 answer reloads
 * the account (`session.refreshUser`), the layout then shows the sign-in or
 * "Access denied". The pages of Administration share this; the account pages
 * only know 401.
 */
import { isApiError } from '#lib/api.ts';
import { session } from '#lib/state/session.svelte.ts';

/** Reloads the account after 401/403 and says so; other errors stay with the caller. */
export function lostAccess(error: unknown): boolean {
  if (!isApiError(error, 401) && !isApiError(error, 403)) return false;
  void session.refreshUser().catch(() => {});
  return true;
}
