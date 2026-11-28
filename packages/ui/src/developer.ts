/**
 * Values of the debug bar (`DebugBar.svelte`): layout, window size and view,
 * from the app's environment (`ErrorEnv`). The log lives in
 * `state/devlog.svelte.ts`, the offline simulation in
 * `state/network.svelte.ts`.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/developer.ts`) only with the
 * debug bar's part: Mail's Developer page values stay with the app. The
 * distance from the bottom edge is a parameter instead of Mail's place above
 * its tab bar on mobile.
 */
import { layoutOf, type ErrorEnv } from './errors.ts';

/** Distance of the debug bar from the bottom edge without a parameter. */
export const DEBUG_BOTTOM = '18px';

export interface DebugBarValues {
  /** "Desktop · 1440×900 · /admin/developer". */
  text: string;
  /** Distance from the bottom edge (CSS length). */
  bottom: string;
}

/** Debug bar: layout, window size and view; `bottom` e.g. above an app's tab bar on mobile. */
export function debugBarValues(env: ErrorEnv, bottom: string = DEBUG_BOTTOM): DebugBarValues {
  return {
    text: `${layoutOf(env)} · ${Math.round(env.width)}×${Math.round(env.height)} · ${env.path}`,
    bottom,
  };
}
