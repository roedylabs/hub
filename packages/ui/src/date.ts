/**
 * Date in digits by date format and time zone, both from the app's settings:
 * `de` "25.10.2026", `iso` "2026-10-25" or `us` "10/25/2026", in the given
 * IANA time zone (without one, or an unknown one, the browser's). For example
 * the short date of a list (`formatShortDate`) or "Released" on an About page.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/date.ts`) without changes
 * to the code; Mail read format and zone from its settings `dateFmt` and `tz`.
 */

const formats = new Map<string, Intl.DateTimeFormat>();

/** Formatter per time zone, cached; an unknown zone counts as the browser's. */
function formatOf(tz: string | undefined): Intl.DateTimeFormat {
  const key = tz ?? '';
  let f = formats.get(key);
  if (!f) {
    const options = { year: 'numeric', month: '2-digit', day: '2-digit' } as const;
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

/** Year, month and day in `tz`; without a valid time zone in the browser's. */
function parts(t: number, tz: string | undefined): { y: string; mo: string; day: string } {
  try {
    const out = formatOf(tz).formatToParts(t);
    const part = (type: Intl.DateTimeFormatPartTypes) => out.find((p) => p.type === type)?.value ?? '';
    return { y: part('year'), mo: part('month'), day: part('day') };
  } catch {
    // Invalid time (RangeError).
    return { y: '', mo: '', day: '' };
  }
}

/** "25.10.2026" (`de`), "2026-10-25" (`iso`) or "10/25/2026" (`us`). */
export function formatDate(t: number, fmt: string, tz?: string): string {
  const { y, mo, day } = parts(t, tz);
  if (fmt === 'iso') return `${y}-${mo}-${day}`;
  if (fmt === 'us') return `${mo}/${day}/${y}`;
  return `${day}.${mo}.${y}`;
}

/**
 * Short date of the list for earlier years: "25.10.24" (`de`) and "10/25/24"
 * (`us`) with a two-digit year, ISO always with the full year ("2024-10-25").
 */
export function formatShortDate(t: number, fmt: string, tz?: string): string {
  const { y, mo, day } = parts(t, tz);
  if (fmt === 'iso') return `${y}-${mo}-${day}`;
  if (fmt === 'us') return `${mo}/${day}/${y.slice(-2)}`;
  return `${day}.${mo}.${y.slice(-2)}`;
}
