/**
 * Shared preferences in `localStorage["roedy.hub.prefs"]` (docs/app-contract.md
 * › Namespaces in the browser): the user's shared settings as the apps get
 * them in `GET /api/v1/session` (`prefs`, `HubPrefs` of `@roedy/ui`). The hub
 * writes the whole object whenever a setting changes (also for guests), so
 * that the apps apply them before their first request and follow them in
 * other tabs (`storage` events, `connectHub()`); its own pages without
 * sign-in read colour scheme, language, font size and animations from it. On
 * sign-out only these four stay. Apps only read it.
 */
import {
  accentOf,
  type Branding,
  browserStorage,
  type HubPrefs,
  PREFS_KEY,
  readHubPrefs,
  resolveFormats,
} from '@roedy/ui';
import type { Lang } from '#lib/locale.ts';
import type { Settings } from '#lib/state/prefs.svelte.ts';

export const SHARED_PREFS_KEY = PREFS_KEY;

/** What the pages without sign-in keep after sign-out. */
const GUEST_KEYS = ['theme', 'language', 'font_size', 'animations'] as const;

/** The stored preferences; unknown or broken values are left out. */
export function readSharedPrefs(storage: Pick<Storage, 'getItem'> | undefined = browserStorage()): Partial<HubPrefs> {
  return readHubPrefs(storage);
}

/**
 * The shared settings of `values` as the apps get them: formats resolved with the UI language `lang`, the accent
 * only if the branding lets users choose it.
 */
export function sharedPrefsOf(values: Settings, lang: Lang, brand: Pick<Branding, 'userAccent' | 'color'>): HubPrefs {
  const auto = resolveFormats(values.region, lang);
  return {
    theme: values.mode ?? 'system',
    accent: brand.userAccent && values.accent !== null ? accentOf(values, brand) : null,
    font_size: values.font_size,
    density: values.density,
    animations: values.animations,
    language: values.lang,
    region: values.region,
    time_zone: values.time_zone,
    time_zone_auto: values.time_zone_auto,
    date_format: values.date_format ?? auto.date_format,
    time_format: values.time_format ?? auto.time_format,
    week_start: values.week_start ?? auto.week_start,
    number_format: values.number_format ?? auto.number_format,
    confirm_delete: values.confirm_delete,
    undo_delay: values.undo_delay,
    shortcuts: values.shortcuts,
    developer: values.developer,
  };
}

function store(prefs: Partial<HubPrefs>, storage: Pick<Storage, 'setItem'> | undefined): void {
  try {
    storage?.setItem(SHARED_PREFS_KEY, JSON.stringify(prefs));
  } catch {
    // Without storage the defaults apply next time.
  }
}

/** Stores `prefs` as the shared preferences (the whole object). */
export function writeSharedPrefs(
  prefs: HubPrefs,
  storage: Pick<Storage, 'setItem'> | undefined = browserStorage(),
): void {
  store(prefs, storage);
}

/** After sign-out: keeps only colour scheme, language, font size and animations for the pages without sign-in. */
export function trimSharedPrefs(storage: Pick<Storage, 'getItem' | 'setItem'> | undefined = browserStorage()): void {
  try {
    if (storage?.getItem(SHARED_PREFS_KEY) == null) return;
  } catch {
    return;
  }
  const current = readSharedPrefs(storage);
  const kept: Partial<HubPrefs> = {};
  for (const key of GUEST_KEYS) {
    if (key in current) (kept as Record<string, unknown>)[key] = current[key];
  }
  store(kept, storage);
}
