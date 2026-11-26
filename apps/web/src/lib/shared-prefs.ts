/**
 * Shared preferences in `localStorage["roedy.hub.prefs"]` (docs/app-contract.md
 * › Namespaces in the browser): `{"theme": "system"|"light"|"dark", "locale":
 * "en"|"de"}`. The hub writes them whenever language or color scheme change
 * (also for guests), so that its own pages without sign-in and the apps can
 * apply both before their first request. Apps only read them.
 */
import { isLocale, type Locale } from '#lib/paraglide/runtime.js';

export const SHARED_PREFS_KEY = 'roedy.hub.prefs';

export type SharedTheme = 'system' | 'light' | 'dark';

export interface SharedPrefs {
  theme?: SharedTheme;
  locale?: Locale;
}

const THEMES: readonly string[] = ['system', 'light', 'dark'];

/** The browser's `localStorage`; `undefined` where the getter throws (storage blocked by a policy). */
function browserStorage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

/** The stored preferences; unknown or broken values are left out. */
export function readSharedPrefs(storage: Pick<Storage, 'getItem'> | undefined = browserStorage()): SharedPrefs {
  let raw: string | null = null;
  try {
    raw = storage?.getItem(SHARED_PREFS_KEY) ?? null;
  } catch {
    // No storage (private window, blocked).
  }
  if (!raw) return {};
  let data: unknown;
  try {
    data = JSON.parse(raw);
  } catch {
    return {};
  }
  if (typeof data !== 'object' || data === null) return {};
  const { theme, locale } = data as Record<string, unknown>;
  const prefs: SharedPrefs = {};
  if (typeof theme === 'string' && THEMES.includes(theme)) prefs.theme = theme as SharedTheme;
  if (typeof locale === 'string' && isLocale(locale)) prefs.locale = locale;
  return prefs;
}

/** Merges `patch` into the stored preferences; `locale: null` forgets the language (it follows the browser). */
export function writeSharedPrefs(
  patch: { theme?: SharedTheme; locale?: Locale | null },
  storage: Pick<Storage, 'getItem' | 'setItem'> | undefined = browserStorage(),
): void {
  const next: SharedPrefs = { ...readSharedPrefs(storage) };
  if (patch.theme) next.theme = patch.theme;
  if (patch.locale === null) delete next.locale;
  else if (patch.locale) next.locale = patch.locale;
  try {
    storage?.setItem(SHARED_PREFS_KEY, JSON.stringify(next));
  } catch {
    // Without storage the defaults apply next time.
  }
}
