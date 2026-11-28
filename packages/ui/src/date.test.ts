// Taken over from Roedy Mail (`apps/web/src/lib/date.test.ts`).
import { afterEach, describe, expect, it } from 'vitest';
import { firstDayOfWeek, formatDate, formatNumber, formatShortDate, formatTime } from './date.ts';
import { connectSettings } from './settings.svelte.ts';

describe('formatDate', () => {
  const t = Date.parse('2026-10-01T20:00:00Z');

  it('writes the date in the chosen format', () => {
    expect(formatDate(t, 'de', 'Europe/Berlin')).toBe('01.10.2026');
    expect(formatDate(t, 'iso', 'Europe/Berlin')).toBe('2026-10-01');
    expect(formatDate(t, 'us', 'Europe/Berlin')).toBe('10/01/2026');
  });

  it('calculates in the time zone of the settings', () => {
    expect(formatDate(t, 'de', 'Asia/Tokyo')).toBe('02.10.2026');
  });

  it('falls back to the browser time zone for an unknown one', () => {
    expect(formatDate(t, 'iso', 'Mars/Olympus')).toMatch(/^2026-10-0[12]$/);
  });
});

describe('formatShortDate', () => {
  const t = Date.parse('2025-12-24T09:00:00Z');

  it('shortens the year in de and us, ISO keeps the full year', () => {
    expect(formatShortDate(t, 'de', 'Europe/Berlin')).toBe('24.12.25');
    expect(formatShortDate(t, 'us', 'Europe/Berlin')).toBe('12/24/25');
    expect(formatShortDate(t, 'iso', 'Europe/Berlin')).toBe('2025-12-24');
  });

  it('returns only the separators instead of an error for an invalid time', () => {
    expect(formatShortDate(NaN, 'iso', 'Europe/Berlin')).toBe('--');
  });
});

describe('formats of the settings', () => {
  const t = Date.parse('2026-10-25T14:57:00Z');

  afterEach(() => connectSettings({}));

  it('writes `dmy` with slashes, also short', () => {
    expect(formatDate(t, 'dmy', 'Europe/Berlin')).toBe('25/10/2026');
    expect(formatShortDate(t, 'dmy', 'Europe/Berlin')).toBe('25/10/26');
  });

  it('writes the time on the 24- or 12-hour clock in the zone', () => {
    expect(formatTime(t, '24h', 'Europe/Berlin')).toBe('15:57');
    expect(formatTime(t, '12h', 'Europe/Berlin')).toBe('3:57 PM');
    expect(formatTime(Date.parse('2026-10-25T23:05:00Z'), '12h', 'Europe/Berlin')).toBe('12:05 AM');
    expect(formatTime(Date.parse('2026-10-25T11:05:00Z'), '12h', 'UTC')).toBe('11:05 AM');
    expect(formatTime(Date.parse('2026-10-25T12:05:00Z'), '12h', 'UTC')).toBe('12:05 PM');
    expect(formatTime(Date.parse('2026-10-25T22:05:00Z'), '24h', 'UTC')).toBe('22:05');
    expect(formatTime(Date.parse('2026-10-25T00:05:00Z'), '24h', 'UTC')).toBe('00:05');
    expect(formatTime(NaN, '24h', 'UTC')).toBe('');
  });

  it('writes numbers with the separators of each format', () => {
    const two = { minimumFractionDigits: 2, maximumFractionDigits: 2 };
    expect(formatNumber(1234567.891, two, '1.234,56')).toBe('1.234.567,89');
    expect(formatNumber(1234567.891, two, '1,234.56')).toBe('1,234,567.89');
    expect(formatNumber(1234.5, two, '1 234,56')).toBe('1 234,50');
    expect(formatNumber(1234.5, two, '1’234.56')).toBe('1’234.50');
    expect(formatNumber(-12, {}, '1.234,56')).toBe('-12');
  });

  it('takes format, zone and week start from the connected settings', () => {
    connectSettings({
      dateFormat: () => 'us',
      timeFormat: () => '12h',
      numberFormat: () => '1.234,56',
      timeZone: () => 'Asia/Tokyo',
      weekStart: () => 'sunday',
    });
    expect(formatDate(t)).toBe('10/25/2026');
    expect(formatShortDate(t)).toBe('10/25/26');
    expect(formatTime(t)).toBe('11:57 PM');
    expect(formatNumber(1000.5)).toBe('1.000,5');
    expect(firstDayOfWeek()).toBe(0);
    expect(firstDayOfWeek('monday')).toBe(1);
  });

  it('follows the language without a region or connected formats', () => {
    connectSettings({ lang: () => 'de', timeZone: () => 'UTC' });
    expect(formatDate(t)).toBe('25.10.2026');
    connectSettings({ lang: () => 'en', timeZone: () => 'UTC' });
    expect(formatDate(t)).toBe('2026-10-25');
    expect(formatTime(t)).toBe('14:57');
  });
});
