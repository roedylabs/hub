import { describe, expect, it, vi } from 'vitest';
import { langChoices, languageValues, type LanguageContext, type LanguageInput } from '#lib/language.ts';
import { RESET0 } from '#lib/state/prefs.svelte.ts';

describe('langChoices', () => {
  it('offers "System" with the browser’s language first, then the languages by their own names', () => {
    const choices = langChoices('de', 'en', () => {});
    expect(choices.map((c) => [c.id, c.code, c.name, c.sub, c.label, c.on])).toEqual([
      ['system', '', 'System', 'Browser: English', 'System – Browser: English', false],
      ['de', 'DE', 'Deutsch', 'German', 'DE Deutsch', true],
      ['en', 'EN', 'English', '', 'EN English', false],
    ]);
  });

  it('selects only "System" while it follows the browser', () => {
    expect(
      langChoices('system', 'de', () => {})
        .filter((c) => c.on)
        .map((c) => c.id),
    ).toEqual(['system']);
  });

  it('saves a different language at once and leaves the chosen one alone', () => {
    const set = vi.fn();
    const [system, de, en] = langChoices('en', 'en', set);
    en!.onClick();
    expect(set).not.toHaveBeenCalled();
    de!.onClick();
    system!.onClick();
    expect(set.mock.calls).toEqual([[{ lang: 'de' }], [{ lang: 'system' }]]);
  });
});

describe('languageValues', () => {
  const NOW = Date.UTC(2026, 10, 1, 10, 0);

  function setup(values: Partial<LanguageInput['values']> = {}, input: Partial<LanguageInput> = {}) {
    const ctx: LanguageContext = { set: vi.fn(), scrollTo: vi.fn(), show: vi.fn(), mark: vi.fn() };
    const v = languageValues(
      {
        values: { ...RESET0, ...values },
        uiLang: 'en',
        browserLang: 'en',
        appName: 'Example Org',
        now: NOW,
        deviceTz: 'Europe/Berlin',
        open: null,
        hi: null,
        isMobile: false,
        ...input,
      },
      ctx,
    );
    return { v, ctx };
  }

  it('returns the cards with a second line only where it differs', () => {
    const { v } = setup({ lang: 'en' }, { browserLang: 'de' });
    expect(v.langOpts.map((l) => [l.id, l.sub, l.hasSub, l.on])).toEqual([
      ['system', 'Browser: Deutsch', true, false],
      ['de', 'German', true, false],
      ['en', '', false, true],
    ]);
    expect(v.systemNote).toBeNull();
  });

  it('notes under "System" that mails use the language of the last sign-in', () => {
    expect(setup({ lang: 'system' }).v.systemNote).toBe(
      'Emails from Example Org use the language of your last sign-in.',
    );
  });

  it('saves a choice through `set`', () => {
    const { v, ctx } = setup();
    v.langOpts[1]!.onClick();
    expect(ctx.set).toHaveBeenCalledWith({ lang: 'de' });
  });

  it('offers "Not set" and the countries; a choice saves the region', () => {
    const { v, ctx } = setup();
    expect(v.selRegion.label).toBe('Not set');
    expect(v.selRegion.opts.map((o) => o.label)).toEqual([
      'Not set',
      'Germany',
      'Austria',
      'Switzerland',
      'France',
      'Italy',
      'Netherlands',
      'Spain',
      'United Kingdom',
      'United States',
    ]);
    v.selRegion.opts[3]!.onClick();
    v.selRegion.opts[0]!.onClick();
    expect(vi.mocked(ctx.set).mock.calls).toEqual([[{ region: 'CH' }], [{ region: null }]]);
  });

  it('resolves "Automatic" with the region, without one with the language', () => {
    expect(setup({ region: 'FR' }).v.fmtPreview.map((p) => p.v)).toEqual(['01/11/2026', '11:00', '12\u00a0345,67']);
    expect(setup({}, { uiLang: 'de' }).v.fmtPreview.map((p) => p.v)).toEqual(['01.11.2026', '11:00', '12.345,67']);
    const own = setup({ region: 'US', date_format: 'iso', number_format: '1.234,56' }).v;
    expect(own.fmtPreview.map((p) => p.v)).toEqual(['2026-11-01', '11:00 AM', '12.345,67']);
    expect(own.selDate.label).toBe('2026-11-01');
    expect(own.selTime.label).toBe('Automatic (12-hour · 11:00 AM)');
    expect(own.selWeek.label).toBe('Automatic (Sunday)');
  });

  it('saves a format and `null` for "Automatic"', () => {
    const { v, ctx } = setup({ date_format: 'us' });
    v.selDate.opts[0]!.onClick();
    v.selTime.opts[2]!.onClick();
    v.selWeek.opts[2]!.onClick();
    v.selNumber.opts[4]!.onClick();
    expect(vi.mocked(ctx.set).mock.calls).toEqual([
      [{ date_format: null }],
      [{ time_format: '12h' }],
      [{ week_start: 'sunday' }],
      [{ number_format: '1\u2019234.56' }],
    ]);
  });

  it('locks the zone while automatic and reports the device zone with the switch', () => {
    const auto = setup({ time_zone: 'Asia/Tokyo' }, { deviceTz: 'America/New_York' });
    expect(auto.v.selTz.disabled).toBe(true);
    // Automatic: the device's zone applies, not the last reported one.
    expect(auto.v.selTz.label).toMatch(/^New York, Toronto/);
    auto.v.tgTzAuto.onClick();
    expect(auto.ctx.set).toHaveBeenCalledWith({ time_zone_auto: false, time_zone: 'America/New_York' });
    const own = setup({ time_zone_auto: false, time_zone: 'Asia/Tokyo' });
    expect(own.v.selTz.disabled).toBe(false);
    expect(own.v.fmtPreview[1]!.v).toBe('19:00');
    own.v.tgTzAuto.onClick();
    expect(own.ctx.set).toHaveBeenCalledWith({ time_zone_auto: true, time_zone: 'Europe/Berlin' });
  });
});
