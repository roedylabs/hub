import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  applyDisplay,
  DEFAULT_PREFS,
  effectiveZone,
  knownPrefs,
  PREFS_KEY,
  readHubPrefs,
  REGION_FORMATS,
  REGIONS,
  regionOfTag,
  resolveFormats,
} from './prefs.ts';

afterEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});

describe('region formats', () => {
  it('gives every region its usual formats', () => {
    expect(Object.keys(REGION_FORMATS)).toEqual([...REGIONS]);
    expect(resolveFormats('DE', 'en')).toEqual({
      date_format: 'de',
      time_format: '24h',
      week_start: 'monday',
      number_format: '1.234,56',
    });
    expect(resolveFormats('CH', 'de').number_format).toBe('1’234.56');
    expect(resolveFormats('FR', 'de')).toMatchObject({ date_format: 'dmy', number_format: '1 234,56' });
    expect(resolveFormats('GB', 'de')).toMatchObject({ date_format: 'dmy', number_format: '1,234.56' });
    expect(resolveFormats('US', 'de')).toEqual({
      date_format: 'us',
      time_format: '12h',
      week_start: 'sunday',
      number_format: '1,234.56',
    });
  });

  it('follows the language without a region', () => {
    expect(resolveFormats(null, 'de')).toMatchObject({ date_format: 'de', number_format: '1.234,56' });
    expect(resolveFormats(null, 'en')).toMatchObject({ date_format: 'iso', number_format: '1,234.56' });
    expect(DEFAULT_PREFS).toMatchObject(resolveFormats(null, 'en'));
  });

  it('takes the region of a browser language tag when it is offered', () => {
    expect(regionOfTag('de-AT')).toBe('AT');
    expect(regionOfTag('en-us')).toBe('US');
    expect(regionOfTag('de_CH')).toBe('CH');
    expect(regionOfTag('zh-Hant-TW')).toBeNull();
    expect(regionOfTag('pt-BR')).toBeNull();
    expect(regionOfTag('de')).toBeNull();
    expect(regionOfTag(undefined)).toBeNull();
  });
});

describe('shared settings', () => {
  it('keeps only known fields with valid values', () => {
    expect(
      knownPrefs({
        theme: 'dark',
        accent: '#12ab9f',
        font_size: 'huge',
        density: 'compact',
        animations: 'yes',
        language: 'system',
        region: null,
        time_zone: 'Asia/Tokyo',
        date_format: 'dmy',
        number_format: '1’234.56',
        undo_delay: 7,
        developer: true,
        other: 1,
      }),
    ).toEqual({
      theme: 'dark',
      accent: '#12ab9f',
      density: 'compact',
      language: 'system',
      region: null,
      time_zone: 'Asia/Tokyo',
      date_format: 'dmy',
      number_format: '1’234.56',
      developer: true,
    });
    expect(knownPrefs({ time_zone: 'Mars/Olympus', region: 'BR', accent: 'teal' })).toEqual({});
    expect(knownPrefs(null)).toEqual({});
    expect(knownPrefs([1])).toEqual({});
  });

  it('reads the copy under `roedy.hub.prefs` leniently', () => {
    // Written out, so that a rename of the constant fails here: the apps read this key (docs/app-contract.md).
    expect(PREFS_KEY).toBe('roedy.hub.prefs');
    expect(readHubPrefs()).toEqual({});
    localStorage.setItem(PREFS_KEY, '{');
    expect(readHubPrefs()).toEqual({});
    localStorage.setItem(PREFS_KEY, JSON.stringify({ theme: 'light', font_size: 'large', locale: 'de' }));
    expect(readHubPrefs()).toEqual({ theme: 'light', font_size: 'large' });
    const broken = {
      getItem: () => {
        throw new Error('blocked');
      },
    };
    expect(readHubPrefs(broken)).toEqual({});
  });

  it('uses the device zone while the zone is automatic', () => {
    vi.spyOn(Intl.DateTimeFormat.prototype, 'resolvedOptions').mockReturnValue({
      timeZone: 'Asia/Tokyo',
    } as Intl.ResolvedDateTimeFormatOptions);
    expect(effectiveZone({ time_zone: 'Europe/Berlin', time_zone_auto: true })).toBe('Asia/Tokyo');
    expect(effectiveZone({ time_zone: 'Europe/Berlin', time_zone_auto: false })).toBe('Europe/Berlin');
  });
});

describe('applyDisplay', () => {
  it('sets font size, density and animations on the root', () => {
    const root = document.createElement('html');
    applyDisplay(root, { fontSize: 'large', density: 'compact', animations: false });
    expect(root.dataset).toMatchObject({ fontSize: 'large', density: 'compact', animations: 'off' });
    applyDisplay(root, { fontSize: 'medium', density: 'normal', animations: true });
    expect(root.dataset.fontSize).toBe('medium');
    expect(root.dataset.density).toBe('normal');
    expect(root.dataset.animations).toBeUndefined();
  });
});
