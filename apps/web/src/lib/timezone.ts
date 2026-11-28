/**
 * Time zones of Account › Language & region: the choices with their names
 * ("Berlin, Vienna, Zurich") and abbreviations ("CET", "GMT+9").
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/timezone.ts`) with the names
 * and abbreviations only; the device's zone comes from `deviceZone()` of
 * `@roedy/ui`.
 */
import { intlLocale } from '#lib/locale.ts';
import { m } from '#lib/paraglide/messages.js';

/** Time zones of Language & region with their names. */
const ZONE_NAMES: Record<string, () => string> = {
  'Europe/Berlin': m.region_tz_berlin,
  'Europe/London': m.region_tz_london,
  'Europe/Athens': m.region_tz_athens,
  'America/New_York': m.region_tz_new_york,
  'America/Los_Angeles': m.region_tz_los_angeles,
  'Asia/Tokyo': m.region_tz_tokyo,
};

/** The time zones of the selection in Language & region, in their order. */
export const ZONES: readonly string[] = Object.keys(ZONE_NAMES);

/** Place of a time zone: "Tokyo, Seoul"; without its own name the place from the IANA name. */
export function zoneLabel(tz: string): string {
  const own = Object.hasOwn(ZONE_NAMES, tz) ? ZONE_NAMES[tz] : undefined;
  return own?.() ?? (tz.split('/').pop() ?? tz).replace(/_/g, ' ');
}

const abbrFormats = new Map<string, Intl.DateTimeFormat | null>();

/** Abbreviation of a time zone at time `now`: "CET" or "GMT+9"; empty if the zone is unknown. */
export function zoneAbbr(tz: string, now: number): string {
  const locale = intlLocale();
  const key = `${locale}|${tz}`;
  let f = abbrFormats.get(key);
  if (f === undefined) {
    try {
      f = new Intl.DateTimeFormat(locale, { timeZone: tz, timeZoneName: 'short' });
    } catch {
      // Unknown time zone: no abbreviation.
      f = null;
    }
    abbrFormats.set(key, f);
  }
  return f?.formatToParts(now).find((p) => p.type === 'timeZoneName')?.value ?? '';
}
