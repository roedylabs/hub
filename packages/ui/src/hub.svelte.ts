/**
 * `connectHub()`: an app behind the hub takes over the user's shared
 * settings (docs/app-contract.md › `GET /api/v1/session`, `prefs`) and the
 * package follows them – language and colour scheme of its texts and
 * `ui.dark`, font size, density and animations on `<html>`, `ui.motion`,
 * `ui.undoMs`, `ui.confirmDelete`, `ui.shortcuts`, `ui.developer` and the
 * formats of `date.ts`.
 *
 * Order: the copy in `localStorage["roedy.hub.prefs"]` first (before the
 * first request, no flash), then `GET /api/v1/session`; changes in another
 * tab of the hub arrive as `storage` events. The hub's own web app connects
 * its state with `connectSettings()` instead.
 */
import { api } from './api.ts';
import {
  applyDisplay,
  browserStorage,
  DEFAULT_PREFS,
  effectiveZone,
  knownPrefs,
  PREFS_KEY,
  readHubPrefs,
  type HubPrefs,
} from './prefs.ts';
import { browserLang, connectSettings, type Lang } from './settings.svelte.ts';

const state = $state<{ prefs: HubPrefs; browserLang: Lang }>({ prefs: { ...DEFAULT_PREFS }, browserLang: 'en' });

/** The shared settings as currently known (the hub's defaults until `connectHub()` has read any). */
export function hubPrefs(): Readonly<HubPrefs> {
  return state.prefs;
}

/** The language the user's choice means here: `system` is the browser's. */
export function hubLang(prefs: Pick<HubPrefs, 'language'> = state.prefs, browser: Lang = browserLang()): Lang {
  return prefs.language === 'system' ? browser : prefs.language;
}

export interface ConnectHubOptions {
  /** Element for `data-font-size`, `data-density` and `data-animations`; default `<html>`, `null` for none. */
  root?: HTMLElement | null;
  /** Storage with the copy; default `localStorage`. */
  storage?: Pick<Storage, 'getItem'>;
  /** Without `GET /api/v1/session` (e.g. the app already loaded it): only the copy and the `storage` events. */
  session?: false | { prefs?: unknown };
}

/**
 * Connects the hub's shared settings; returns the disconnect (listeners and effect end, the package's defaults
 * apply again). Fields the hub does not send or that this version does not know keep the hub's defaults.
 */
export function connectHub(options: ConnectHubOptions = {}): () => void {
  const storage = options.storage ?? browserStorage();
  const root = options.root === undefined ? globalThis.document?.documentElement : options.root;
  const take = (prefs: Partial<HubPrefs>) => {
    state.prefs = { ...DEFAULT_PREFS, ...prefs };
  };
  state.browserLang = browserLang();
  take(readHubPrefs(storage));

  connectSettings({
    lang: () => hubLang(state.prefs, state.browserLang),
    mode: () => state.prefs.theme,
    fontSize: () => state.prefs.font_size,
    density: () => state.prefs.density,
    animations: () => state.prefs.animations,
    region: () => state.prefs.region,
    timeZone: () => effectiveZone(state.prefs),
    dateFormat: () => state.prefs.date_format,
    timeFormat: () => state.prefs.time_format,
    weekStart: () => state.prefs.week_start,
    numberFormat: () => state.prefs.number_format,
    confirmDelete: () => state.prefs.confirm_delete,
    undoDelay: () => state.prefs.undo_delay,
    shortcuts: () => state.prefs.shortcuts,
    developer: () => state.prefs.developer,
  });

  let live = true;
  const onStorage = (event: StorageEvent) => {
    if (event.key === PREFS_KEY || event.key === null) take(readHubPrefs(storage));
  };
  const onLanguage = () => {
    state.browserLang = browserLang();
  };
  globalThis.addEventListener?.('storage', onStorage);
  globalThis.addEventListener?.('languagechange', onLanguage);

  const stop = root
    ? $effect.root(() => {
        $effect(() => {
          const { font_size, density, animations } = state.prefs;
          applyDisplay(root, { fontSize: font_size, density, animations });
        });
      })
    : () => {};

  if (options.session !== false) {
    const session = options.session
      ? Promise.resolve(options.session)
      : api<{ prefs?: unknown } | null>('GET', 'session');
    session.then(
      (body) => {
        if (live && body && typeof body.prefs === 'object' && body.prefs !== null) take(knownPrefs(body.prefs));
      },
      // Signed out or no connection: the copy stays.
      () => {},
    );
  }

  return () => {
    live = false;
    globalThis.removeEventListener?.('storage', onStorage);
    globalThis.removeEventListener?.('languagechange', onLanguage);
    stop();
    connectSettings({});
  };
}
