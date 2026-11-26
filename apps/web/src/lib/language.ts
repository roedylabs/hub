/**
 * The interface languages to choose from: "System" (follows the browser,
 * `prefs.browserLang`) and the languages themselves, as cards (Account ›
 * Language, `languageValues`) or as the segment of the account menu
 * (`langSeg`). A choice applies at once (`prefs.set`).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/language.ts`) with the
 * language only: the hub stores no region, time zone or formats.
 */
import { m } from '#lib/paraglide/messages.js';
import type { Locale } from '#lib/paraglide/runtime.js';
import type { Settings } from '#lib/state/prefs.svelte.ts';

interface Lang {
  id: Locale;
  code: string;
  /** Name in the language itself ("Deutsch", "English"). */
  name: () => string;
  /** Name in the UI language; only shown if it differs from `name` ("German" under "Deutsch"). */
  sub: () => string;
}

/** Languages (`LANGS`). */
const LANGS: readonly Lang[] = [
  { id: 'de', code: 'DE', name: m.region_lang_de, sub: m.region_lang_de_sub },
  { id: 'en', code: 'EN', name: m.region_lang_en, sub: m.region_lang_en_sub },
];

/** Name of a language in itself ("Deutsch", "English"). */
function langName(lang: Locale): string {
  return lang === 'de' ? m.region_lang_de() : m.region_lang_en();
}

/** Language as a card (page) or segment (account menu, `langSeg`). */
export interface LangChoice {
  id: Settings['lang'];
  /** Code on the tile and in the segment ("DE"); empty for "System", which shows `ti-world`. */
  code: string;
  name: string;
  /**
   * Second line: the name in the UI language if it differs ("Deutsch" – "German"), for "System" always the
   * browser's language ("Browser: English"); otherwise empty.
   */
  sub: string;
  /** Name in the account menu: code and name ("EN English"), for "System" with the browser's language. */
  label: string;
  on: boolean;
  onClick: () => void;
}

/**
 * Languages with selection, "System" first (follows the browser, `browser`); a change applies at once
 * (`prefs.set`), without undo. With `system` only "System" is chosen, not the language it resolves to.
 */
export function langChoices(
  lang: Settings['lang'],
  browser: Locale,
  set: (patch: Partial<Settings>) => void,
): LangChoice[] {
  const pick = (id: Settings['lang']) => () => {
    if (id !== lang) set({ lang: id });
  };
  const browserName = langName(browser);
  return [
    {
      id: 'system',
      code: '',
      name: m.region_lang_system(),
      sub: m.region_lang_system_sub({ lang: browserName }),
      label: m.account_language_system({ lang: browserName }),
      on: lang === 'system',
      onClick: pick('system'),
    },
    ...LANGS.map((l) => {
      const name = l.name();
      const sub = l.sub();
      return {
        id: l.id,
        code: l.code,
        name,
        sub: sub === name ? '' : sub,
        label: `${l.code} ${name}`,
        on: l.id === lang,
        onClick: pick(l.id),
      };
    }),
  ];
}

/** Language as a card of the Language page (`langOpts`), "System" with `ti-world` instead of a code. */
export interface LangOption extends LangChoice {
  hasSub: boolean;
}

export interface LanguageValues {
  langOpts: LangOption[];
  /** Note below the cards while "System" is chosen: mails use the language of the last sign-in. */
  systemNote: string | null;
}

export interface LanguageInput {
  lang: Settings['lang'];
  /** Language of the browser (`prefs.browserLang`), shown under "System". */
  browserLang: Locale;
  /** Name of the hub in the note under "System" (`session.appName`). */
  appName: string;
}

/**
 * Values of the Account › Language page: the language cards and, while "System" is chosen, the note that the
 * server's mails then use the language of the last sign-in (`crate::settings` of the server). Roedy Mail's
 * region, time zone and formats are left out: the hub stores only the language.
 */
export function languageValues(input: LanguageInput, set: (patch: Partial<Settings>) => void): LanguageValues {
  return {
    langOpts: langChoices(input.lang, input.browserLang, set).map((l) => ({ ...l, hasSub: !!l.sub })),
    systemNote: input.lang === 'system' ? m.region_lang_system_mail({ app: input.appName }) : null,
  };
}
