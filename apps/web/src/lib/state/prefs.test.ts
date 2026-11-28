import { afterEach, describe, expect, it, vi } from 'vitest';
import { formatDate, settings, ui } from '@roedy/ui';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { knownSettings, prefs, RESET0 } from '#lib/state/prefs.svelte.ts';

const KEY = 'roedy.hub.prefs';

function stored(json: Record<string, unknown> = {}, version = 1) {
  return { version, settings: { ...RESET0, ...json } };
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  localStorage.clear();
  prefs.reset();
});

describe('prefs', () => {
  it('drops values the hub does not offer', () => {
    expect(
      knownSettings({
        font_size: 'huge',
        density: 'compact',
        region: 'BR',
        time_zone: 'Asia/Tokyo',
        date_format: null,
        time_format: '13h',
        number_format: '1’234.56',
        undo_delay: 5,
        developer: 'yes',
      }),
    ).toEqual({
      density: 'compact',
      time_zone: 'Asia/Tokyo',
      date_format: null,
      number_format: '1’234.56',
      undo_delay: 5,
    });
  });

  it('gives a new account the browser’s region together with its language', async () => {
    vi.spyOn(navigator, 'languages', 'get').mockReturnValue(['de-AT', 'de']);
    const { calls } = mockApi({
      'GET settings': [{ body: stored({}, 0) }],
      'PUT settings': [({ body }) => ({ body: stored((body as { settings: Record<string, unknown> }).settings) })],
    });
    prefs.set({ lang: 'de' });
    await prefs.load();
    await prefs.settled();
    expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([
      {
        version: 0,
        settings: {
          lang: 'de',
          region: 'AT',
          date_format: null,
          time_format: null,
          week_start: null,
          number_format: null,
        },
      },
    ]);
    expect(prefs.formats).toEqual({
      date_format: 'de',
      time_format: '24h',
      week_start: 'monday',
      number_format: '1.234,56',
    });
  });

  it('leaves the region of a stored account alone and reports a new device zone', async () => {
    vi.spyOn(navigator, 'languages', 'get').mockReturnValue(['en-US']);
    vi.spyOn(Intl.DateTimeFormat.prototype, 'resolvedOptions').mockReturnValue({
      timeZone: 'Asia/Tokyo',
    } as Intl.ResolvedDateTimeFormatOptions);
    const { calls } = mockApi({
      'GET settings': [{ body: stored() }],
      'PUT settings': [{ body: stored({ time_zone: 'Asia/Tokyo' }, 2) }],
    });
    await prefs.load();
    await prefs.settled();
    expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([
      { version: 1, settings: { time_zone: 'Asia/Tokyo' } },
    ]);
    expect(prefs.values.region).toBeNull();
    expect(prefs.timeZone).toBe('Asia/Tokyo');
  });

  it('reports no zone while the zone is chosen', async () => {
    vi.spyOn(Intl.DateTimeFormat.prototype, 'resolvedOptions').mockReturnValue({
      timeZone: 'Asia/Tokyo',
    } as Intl.ResolvedDateTimeFormatOptions);
    const { calls } = mockApi({
      'GET settings': [{ body: stored({ time_zone_auto: false, time_zone: 'America/New_York' }) }],
    });
    await prefs.load();
    await prefs.settled();
    expect(calls.map((c) => c.method)).toEqual(['GET']);
    expect(prefs.timeZone).toBe('America/New_York');
  });

  it('resets the formats with a new region, not with the same one', () => {
    prefs.set({ region: 'DE', date_format: 'iso', time_format: '12h' });
    expect(prefs.values).toMatchObject({ region: 'DE', date_format: 'iso', time_format: '12h' });
    prefs.set({ region: 'DE' });
    expect(prefs.values.date_format).toBe('iso');
    prefs.set({ region: 'US' });
    expect(prefs.values).toMatchObject({ date_format: null, time_format: null, week_start: null, number_format: null });
    expect(prefs.formats.date_format).toBe('us');
    prefs.set({ region: 'GB', number_format: '1.234,56' });
    expect(prefs.values.number_format).toBe('1.234,56');
  });

  it('connects every setting to `@roedy/ui` and shares it with the apps', () => {
    prefs.set({
      region: 'US',
      animations: false,
      undo_delay: 3,
      confirm_delete: false,
      shortcuts: false,
      developer: true,
      time_zone_auto: false,
      time_zone: 'Asia/Tokyo',
    });
    expect(settings().dateFormat()).toBe('us');
    expect(settings().timeZone()).toBe('Asia/Tokyo');
    expect(formatDate(Date.UTC(2026, 10, 1, 20, 0))).toBe('11/02/2026');
    expect([ui.undoMs, ui.motion, ui.confirmDelete, ui.shortcuts, ui.developer]).toEqual([
      3000,
      false,
      false,
      false,
      true,
    ]);
    expect(JSON.parse(localStorage.getItem(KEY) ?? '')).toMatchObject({
      region: 'US',
      date_format: 'us',
      time_format: '12h',
      week_start: 'sunday',
      undo_delay: 3,
      developer: true,
    });
  });

  it('keeps colour scheme, language, font size and animations for the pages without sign-in', () => {
    prefs.set({ mode: 'dark', lang: 'de', font_size: 'large', animations: false, developer: true });
    prefs.reset();
    expect(JSON.parse(localStorage.getItem(KEY) ?? '')).toEqual({
      theme: 'dark',
      language: 'de',
      font_size: 'large',
      animations: false,
    });
    expect(prefs.values).toMatchObject({
      mode: 'dark',
      lang: 'de',
      font_size: 'large',
      animations: false,
      developer: false,
    });
  });
});
