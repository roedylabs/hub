/**
 * Times of the audit log: days and times of the entries and the date of the
 * export file, in a time zone (the browser's; the hub has no time zone
 * setting) and the UI language.
 *
 * Taken over from Roedy Mail (`dtf`, `dayNumber`, `clock` and `dayLabel` in
 * `apps/web/src/lib/mail-list.ts`, `formatDate` in `#lib/date.ts`) without
 * Mail's time format setting, with the year on days of another year.
 */
import { intlLocale, type Lang } from '#lib/locale.ts';
import { m } from '#lib/paraglide/messages.js';

/** Time context of the list: now, time zone (empty = the browser's) and UI language. */
export interface AuditTime {
  now: number;
  /** IANA time zone; empty or unknown = the browser's. */
  tz: string;
  lang: Lang;
}

const DAY = 86_400_000;
const formats = new Map<string, Intl.DateTimeFormat>();

/** Formatter per language, time zone and options, cached; an unknown zone counts as the browser's. */
function dtf(time: Pick<AuditTime, 'tz' | 'lang'>, options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const locale = intlLocale(time.lang);
  const key = `${locale}|${time.tz}|${JSON.stringify(options)}`;
  let f = formats.get(key);
  if (!f) {
    try {
      f = new Intl.DateTimeFormat(locale, { ...options, timeZone: time.tz || undefined });
    } catch {
      f = new Intl.DateTimeFormat(locale, options);
    }
    formats.set(key, f);
  }
  return f;
}

/** Calendar day of `t` in the time zone as a number (days since 1970). */
export function dayNumber(t: number, time: Pick<AuditTime, 'tz' | 'lang'>): number {
  const parts = dtf(time, { year: 'numeric', month: 'numeric', day: 'numeric' }).formatToParts(t);
  const n = (type: Intl.DateTimeFormatPartTypes) => Number(parts.find((p) => p.type === type)?.value);
  return Math.round(Date.UTC(n('year'), n('month') - 1, n('day')) / DAY);
}

function yearOf(t: number, time: Pick<AuditTime, 'tz' | 'lang'>): string {
  return dtf(time, { year: 'numeric' }).format(t);
}

/** Time of day in the language ("09:26"). */
export function clock(t: number, time: Pick<AuditTime, 'tz' | 'lang'>): string {
  return dtf(time, { hour: '2-digit', minute: '2-digit' }).format(t);
}

/**
 * Heading of a day: today, yesterday, otherwise weekday and date ("Monday 2 November"); with the year if it is
 * not the current one (a retention of one year reaches the same date a year ago).
 */
export function dayLabel(t: number, time: AuditTime): string {
  const k = dayNumber(t, time) - dayNumber(time.now, time);
  if (k === 0) return m.audit_today();
  if (k === -1) return m.audit_yesterday();
  const year = yearOf(t, time) === yearOf(time.now, time) ? {} : { year: 'numeric' as const };
  return dtf(time, { weekday: 'long', day: 'numeric', month: 'long', ...year }).format(t);
}

/** Wall clock of `t` in the time zone, as milliseconds since 1970 as if it were UTC. */
function wallClock(t: number, tz: string): number {
  const parts = dtf(
    { tz, lang: 'en' },
    {
      year: 'numeric',
      month: 'numeric',
      day: 'numeric',
      hour: 'numeric',
      minute: 'numeric',
      second: 'numeric',
      hourCycle: 'h23',
    },
  ).formatToParts(t);
  const n = (type: Intl.DateTimeFormatPartTypes) => Number(parts.find((p) => p.type === type)?.value);
  const ms = ((t % 1000) + 1000) % 1000;
  return Date.UTC(n('year'), n('month') - 1, n('day'), n('hour'), n('minute'), n('second')) + ms;
}

/**
 * Milliseconds until the next day begins in the time zone, so that "Today" and "Yesterday" move on at midnight: the
 * first instant with a later local date, found by bisection, so that a change to or from summer time before then
 * and a skipped midnight (the clock jumps from 23:59:59 to 01:00) are taken into account. At least one second, the
 * caller checks again.
 */
export function untilNextDay(now: number, tz: string): number {
  const date = (t: number) => Math.floor(wallClock(t, tz) / DAY);
  const today = date(now);
  // `lo` is still today, `hi` already a later date (no time zone changes its offset by a day).
  let lo = now;
  let hi = now + 2 * DAY;
  while (hi - lo > 1) {
    const mid = lo + Math.floor((hi - lo) / 2);
    if (date(mid) > today) hi = mid;
    else lo = mid;
  }
  return Math.max(1000, hi - now);
}

/** Date in the time zone as `YYYY-MM-DD`. */
export function isoDate(t: number, tz: string): string {
  const parts = dtf({ tz, lang: 'en' }, { year: 'numeric', month: '2-digit', day: '2-digit' }).formatToParts(t);
  const part = (type: Intl.DateTimeFormatPartTypes) => parts.find((p) => p.type === type)?.value ?? '';
  return `${part('year')}-${part('month')}-${part('day')}`;
}

/** File name with the date in the time zone (`audit-log-YYYY-MM-DD.csv`); the server only knows UTC. */
export function exportName(now: number, tz: string): string {
  return m.audit_file_name({ date: isoDate(now, tz) });
}

/** The browser's time zone (the hub has no setting of its own). */
export function browserTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone ?? '';
  } catch {
    return '';
  }
}
