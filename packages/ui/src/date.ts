/**
 * Dates, times and numbers in the formats and time zone of the settings
 * (`connectSettings`, `connectHub`; `prefs.ts`): date in digits `de`
 * "25.10.2026", `dmy` "25/10/2026", `iso` "2026-10-25" or `us` "10/25/2026";
 * time "15:57" or "3:57 PM"; numbers with the chosen separators; the first
 * day of the week. Each helper takes format and zone as optional parameters
 * (an unknown zone counts as the browser's).
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/date.ts`), which read format
 * and zone from its settings `dateFmt` and `tz`; `dmy`, the defaults from the
 * settings, `formatTime`, `formatNumber` and `firstDayOfWeek` are new.
 */
import type { NumberFormat, TimeFormat, WeekStart } from './prefs.ts';
import { settings } from './settings.svelte.ts';

const formats = new Map<string, Intl.DateTimeFormat>();

/** Formatter per time zone, cached; an unknown zone counts as the browser's. */
function formatOf(tz: string | undefined): Intl.DateTimeFormat {
  const key = tz ?? '';
  let f = formats.get(key);
  if (!f) {
    const options = {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      hourCycle: 'h23',
    } as const;
    try {
      f = new Intl.DateTimeFormat('en-US', { ...options, timeZone: tz || undefined });
    } catch {
      // Unknown time zone (RangeError): local time of the browser.
      f = new Intl.DateTimeFormat('en-US', options);
    }
    formats.set(key, f);
  }
  return f;
}

/** Year, month, day, hour and minute in `tz`; without a valid time zone in the browser's. */
function parts(t: number, tz: string | undefined): { y: string; mo: string; day: string; h: string; mi: string } {
  try {
    const out = formatOf(tz).formatToParts(t);
    const part = (type: Intl.DateTimeFormatPartTypes) => out.find((p) => p.type === type)?.value ?? '';
    return { y: part('year'), mo: part('month'), day: part('day'), h: part('hour'), mi: part('minute') };
  } catch {
    // Invalid time (RangeError).
    return { y: '', mo: '', day: '', h: '', mi: '' };
  }
}

/** "25.10.2026" (`de`; any unknown format too), "25/10/2026" (`dmy`), "2026-10-25" (`iso`) or "10/25/2026" (`us`). */
export function formatDate(
  t: number,
  fmt: string = settings().dateFormat(),
  tz: string | undefined = settings().timeZone(),
): string {
  const { y, mo, day } = parts(t, tz);
  if (fmt === 'iso') return `${y}-${mo}-${day}`;
  if (fmt === 'us') return `${mo}/${day}/${y}`;
  if (fmt === 'dmy') return `${day}/${mo}/${y}`;
  return `${day}.${mo}.${y}`;
}

/**
 * Short date of a list for earlier years: "25.10.24" (`de`), "25/10/24" (`dmy`) and "10/25/24" (`us`) with a
 * two-digit year, ISO always with the full year ("2024-10-25").
 */
export function formatShortDate(
  t: number,
  fmt: string = settings().dateFormat(),
  tz: string | undefined = settings().timeZone(),
): string {
  const { y, mo, day } = parts(t, tz);
  if (fmt === 'iso') return `${y}-${mo}-${day}`;
  if (fmt === 'us') return `${mo}/${day}/${y.slice(-2)}`;
  if (fmt === 'dmy') return `${day}/${mo}/${y.slice(-2)}`;
  return `${day}.${mo}.${y.slice(-2)}`;
}

/** "15:57" (`24h`) or "3:57 PM" (`12h`, in every language). */
export function formatTime(
  t: number,
  fmt: TimeFormat = settings().timeFormat(),
  tz: string | undefined = settings().timeZone(),
): string {
  const { h, mi } = parts(t, tz);
  if (!h) return '';
  if (fmt === '24h') return `${h}:${mi}`;
  const hour = Number(h);
  return `${hour % 12 || 12}:${mi} ${hour < 12 ? 'AM' : 'PM'}`;
}

/**
 * A number with the separators of `fmt` ("1.234,56", "1,234.56", "1 234,56" or "1’234.56"); `options` as for
 * `Intl.NumberFormat` (fraction digits, percent …), always with grouping.
 */
export function formatNumber(
  n: number,
  options: Intl.NumberFormatOptions = {},
  fmt: NumberFormat = settings().numberFormat(),
): string {
  const group = fmt.charAt(1);
  const decimal = fmt.charAt(5);
  return new Intl.NumberFormat('en-US', { ...options, useGrouping: true })
    .format(n)
    .replace(/[,.]/g, (c) => (c === ',' ? group : decimal));
}

/** First day of the week as in `Date#getDay()`: 1 Monday, 0 Sunday. */
export function firstDayOfWeek(start: WeekStart = settings().weekStart()): 0 | 1 {
  return start === 'sunday' ? 0 : 1;
}
