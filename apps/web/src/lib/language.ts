/**
 * Values of the Account › Language & region page: language (`langOpts`,
 * "System" and the languages, also the segment of the account menu,
 * `langChoices`), country or region (`selRegion`, with "Not set"), time zone
 * (`tgTzAuto`, `selTz`) and formats (`selDate`, `selTime`, `selWeek`,
 * `selNumber` with the preview `fmtPreview`). Each format offers
 * "Automatic" first – the region's format, without a region the language's
 * (`resolveFormats` of `@roedy/ui`) – and a new region resets them to it
 * (`prefs.set`). Changes go through `prefs.set()` at once.
 *
 * With an automatic time zone the device's applies (`deviceTz`); `prefs`
 * reports it to the server so that the hub's mails and the apps' backends
 * use it. The settings apply in the hub and, through the shared settings, in
 * every app.
 *
 * The lists are select lists: on desktop a popover, on mobile a sheet
 * (`ui.pick`); both hang on the menu `pick`, the page remembers which one is
 * open (`open`).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/language.ts`) with "Not set"
 * for the region, "Automatic" for the formats, the format `dmy` and the
 * number format with an apostrophe; formats are select lists instead of
 * segments.
 */
import {
  DATE_FORMATS,
  formatDate,
  formatNumber,
  formatTime,
  NUMBER_FORMATS,
  REGIONS,
  resolveFormats,
  TIME_FORMATS,
  WEEK_STARTS,
  type Region,
} from '@roedy/ui';
import { m } from '#lib/paraglide/messages.js';
import type { Locale } from '#lib/paraglide/runtime.js';
import { selectValues as select, type SelectValues, type SelOption } from '#lib/select.ts';
import type { Settings } from '#lib/state/prefs.svelte.ts';
import { ZONES, zoneAbbr, zoneLabel } from '#lib/timezone.ts';

/** Sections of the page (`data-sec`, `langTabs`). */
export type LanguageSection = 'lang' | 'region' | 'formats';

/** Select lists of the page. */
export type SelectKey = 'region' | 'tz' | 'date' | 'time' | 'week' | 'number';

export type { SelectValues };

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

/** Language as a card of the page (`langOpts`), "System" with `ti-world` instead of a code. */
export interface LangOption extends LangChoice {
  hasSub: boolean;
}

const REGION_LABELS: Record<Region, () => string> = {
  DE: m.region_country_de,
  AT: m.region_country_at,
  CH: m.region_country_ch,
  FR: m.region_country_fr,
  IT: m.region_country_it,
  NL: m.region_country_nl,
  ES: m.region_country_es,
  GB: m.region_country_gb,
  US: m.region_country_us,
};

/** Name of a country or region ("Germany"); unknown codes as they are. */
export function regionLabel(region: string): string {
  return Object.hasOwn(REGION_LABELS, region) ? REGION_LABELS[region as Region]() : region;
}

const WORDS: Record<(typeof TIME_FORMATS)[number] | (typeof WEEK_STARTS)[number], () => string> = {
  '24h': m.region_time_24,
  '12h': m.region_time_12,
  monday: m.region_week_monday,
  sunday: m.region_week_sunday,
};

/** Example number of the number formats ("12.345,67"). */
const EXAMPLE = 12_345.67;

/** Value of "Automatic" in the select lists (the setting is `null`). */
export const AUTO = 'auto';

export interface LanguageValues {
  langTabs: { label: string; onClick: () => void }[];
  langOpts: LangOption[];
  /** Note below the cards while "System" is chosen: mails use the language of the last sign-in. */
  systemNote: string | null;
  selRegion: SelectValues;
  tgTzAuto: { on: boolean; onClick: () => void };
  selTz: SelectValues;
  fmtPreview: { k: string; v: string }[];
  selDate: SelectValues;
  selTime: SelectValues;
  selWeek: SelectValues;
  selNumber: SelectValues;
}

export interface LanguageInput {
  values: Pick<
    Settings,
    'lang' | 'region' | 'time_zone_auto' | 'time_zone' | 'date_format' | 'time_format' | 'week_start' | 'number_format'
  >;
  /** Language of the interface (`prefs.lang`): "Automatic" without a region follows it. */
  uiLang: Locale;
  /** Language of the browser (`prefs.browserLang`), shown under "System". */
  browserLang: Locale;
  /** Name of the hub in the note under "System" (`session.appName`). */
  appName: string;
  now: number;
  /** Time zone of the device (`deviceZone()`); `undefined` if it reports no usable one. */
  deviceTz: string | undefined;
  /** Open select list (only while `ui.menu` is `pick`). */
  open: SelectKey | null;
  /** Marked row of the open list; `null` = the chosen one. */
  hi: number | null;
  isMobile: boolean;
}

export interface LanguageContext {
  set: (patch: Partial<Settings>) => void;
  scrollTo: (section: LanguageSection) => void;
  /** Open or close (`null`) a select list; the marking starts anew. */
  show: (key: SelectKey | null) => void;
  /** Mark a row (`null` = none). */
  mark: (i: number | null) => void;
}

/** Options of a format: "Automatic (example)" first, then the formats with their example. */
function formatOpts<T extends string>(formats: readonly T[], auto: T, example: (format: T) => string): SelOption[] {
  return [
    { v: AUTO, l: m.region_fmt_auto_sub({ value: example(auto) }) },
    ...formats.map((f) => ({ v: f, l: example(f) })),
  ];
}

export function languageValues(input: LanguageInput, ctx: LanguageContext): LanguageValues {
  const { values, now } = input;
  const tz = values.time_zone_auto ? (input.deviceTz ?? values.time_zone) : values.time_zone;
  // The zone that applies comes first if it is not among the choices.
  const zones = ZONES.includes(tz) ? ZONES : [tz, ...ZONES];
  const tzOpts = zones.map((v) => {
    const l = zoneLabel(v);
    const sub = zoneAbbr(v, now);
    return { v, l, tl: sub ? `${l} (${sub})` : l, sub };
  });
  const auto = resolveFormats(values.region, input.uiLang);
  const date = values.date_format ?? auto.date_format;
  const time = values.time_format ?? auto.time_format;
  const number = values.number_format ?? auto.number_format;
  const state = { open: input.open, hi: input.hi, isMobile: input.isMobile };
  const sel = (
    key: SelectKey,
    title: string,
    opts: readonly SelOption[],
    cur: string,
    set: (v: string) => void,
    disabled = false,
  ) => select(key, title, opts, cur, set, disabled, state, ctx);
  const fmt = <K extends 'date_format' | 'time_format' | 'week_start' | 'number_format'>(key: K) => ({
    cur: values[key] ?? AUTO,
    set: (v: string) => ctx.set({ [key]: v === AUTO ? null : v }),
  });
  const dateFmt = fmt('date_format');
  const timeFmt = fmt('time_format');
  const weekFmt = fmt('week_start');
  const numberFmt = fmt('number_format');
  return {
    langTabs: [
      { label: m.region_tab_lang(), onClick: () => ctx.scrollTo('lang') },
      { label: m.region_tab_region(), onClick: () => ctx.scrollTo('region') },
      { label: m.region_tab_formats(), onClick: () => ctx.scrollTo('formats') },
    ],
    langOpts: langChoices(values.lang, input.browserLang, ctx.set).map((l) => ({ ...l, hasSub: !!l.sub })),
    systemNote: values.lang === 'system' ? m.region_lang_system_mail({ app: input.appName }) : null,
    selRegion: sel(
      'region',
      m.region_country_title(),
      [{ v: '', l: m.region_country_none() }, ...REGIONS.map((v) => ({ v, l: regionLabel(v) }))],
      values.region ?? '',
      (v) => ctx.set({ region: v === '' ? null : (v as Region) }),
    ),
    tgTzAuto: {
      on: values.time_zone_auto,
      // Switching off keeps the zone of the device chosen, switching on reports it.
      onClick: () => ctx.set({ time_zone_auto: !values.time_zone_auto, time_zone: input.deviceTz ?? values.time_zone }),
    },
    selTz: sel('tz', m.region_tz_title(), tzOpts, tz, (v) => ctx.set({ time_zone: v }), values.time_zone_auto),
    fmtPreview: [
      { k: m.region_preview_date(), v: formatDate(now, date, tz) },
      { k: m.region_preview_time(), v: formatTime(now, time, tz) },
      { k: m.region_preview_number(), v: formatNumber(EXAMPLE, { minimumFractionDigits: 2 }, number) },
    ],
    selDate: sel(
      'date',
      m.region_date_fmt(),
      formatOpts(DATE_FORMATS, auto.date_format, (f) => formatDate(now, f, tz)),
      dateFmt.cur,
      dateFmt.set,
    ),
    selTime: sel(
      'time',
      m.region_time_fmt(),
      formatOpts(TIME_FORMATS, auto.time_format, (f) => `${WORDS[f]()} · ${formatTime(now, f, tz)}`),
      timeFmt.cur,
      timeFmt.set,
    ),
    selWeek: sel(
      'week',
      m.region_week_start(),
      formatOpts(WEEK_STARTS, auto.week_start, (f) => WORDS[f]()),
      weekFmt.cur,
      weekFmt.set,
    ),
    selNumber: sel(
      'number',
      m.region_num_fmt(),
      formatOpts(NUMBER_FORMATS, auto.number_format, (f) => formatNumber(EXAMPLE, { minimumFractionDigits: 2 }, f)),
      numberFmt.cur,
      numberFmt.set,
    ),
  };
}
