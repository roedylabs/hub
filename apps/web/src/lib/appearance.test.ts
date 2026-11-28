// Taken over from Roedy Mail (`apps/web/src/lib/appearance.test.ts`) without the sidebar and the mailbox design.
import { describe, expect, it, vi } from 'vitest';
import { appearanceValues, type AppearanceContext, type AppearanceInput } from '#lib/appearance.ts';
import { RESET0 } from '#lib/state/prefs.svelte.ts';

function setup(patch: Partial<AppearanceInput> = {}) {
  const ctx: AppearanceContext = { set: vi.fn(), scrollTo: vi.fn() };
  const input: AppearanceInput = {
    values: { ...RESET0 },
    brand: { userAccent: true, color: 'apricot' },
    ...patch,
  };
  return { ctx, v: appearanceValues(input, ctx) };
}

describe('Appearance', () => {
  it('shows the defaults like RESET0: the system’s colour scheme and the brand colour', () => {
    const { v } = setup();
    expect(v.modeCards.map((c) => [c.label, c.on])).toEqual([
      ['Light', false],
      ['Dark', false],
      ['System', true],
    ]);
    expect(v.accentOpts.find((a) => a.on)?.name).toBe('apricot');
    expect(v.accLocked).toBe(false);
  });

  it('returns preview and icon per mode from MODE_PREV', () => {
    const { v } = setup();
    expect(v.modeCards[0]).toMatchObject({ icon: 'ti-sun', panel: '#fffdfa', inkc: '#2b221c' });
    expect(v.modeCards[2]).toMatchObject({ icon: 'ti-device-desktop', line: 'rgba(150,130,110,.45)' });
  });

  it('saves every choice via prefs', () => {
    const { v, ctx } = setup();
    v.modeCards[1]!.onClick();
    v.accentOpts.find((a) => a.name === 'sky')!.onClick();
    expect(vi.mocked(ctx.set).mock.calls).toEqual([[{ mode: 'dark' }], [{ accent: 'sky' }]]);
  });

  it('locks the accent colour without userAccent: brand colour active, click without effect', () => {
    const { v, ctx } = setup({
      values: { ...RESET0, accent: 'rose' },
      brand: { userAccent: false, color: 'sage' },
    });
    expect(v.accLocked).toBe(true);
    expect(v.accentOpts.find((a) => a.on)?.name).toBe('sage');
    v.accentOpts[0]!.onClick();
    expect(ctx.set).not.toHaveBeenCalled();
  });

  it('a custom accent selects no swatch, a click on one replaces it', () => {
    const { v, ctx } = setup({ values: { ...RESET0, accent: '#336699' } });
    expect(v.accentOpts.filter((a) => a.on)).toEqual([]);
    v.accentOpts.find((a) => a.name === 'sky')!.onClick();
    expect(ctx.set).toHaveBeenCalledWith({ accent: 'sky' });
  });

  it('"More": opens the editor with the active accent, Apply saves the custom colour', () => {
    const { v, ctx } = setup();
    expect(v.accentMore).toMatchObject({ hex: null, ink: null, on: false, accent: 'apricot' });
    v.accentMore.apply('#3a7bd5');
    expect(ctx.set).toHaveBeenCalledWith({ accent: '#3a7bd5' });
  });

  it('"More" shows an active custom accent like a named tile', () => {
    const custom = setup({ values: { ...RESET0, accent: '#3a7bd5' } }).v.accentMore;
    expect(custom).toMatchObject({ hex: '#3a7bd5', on: true, accent: '#3a7bd5' });
    expect(custom.ink).toMatch(/^#[0-9a-f]{6}$/);
    expect(setup({ values: { ...RESET0, accent: 'sky' } }).v.accentMore).toMatchObject({ hex: null, on: false });
  });

  it('"More" is locked like the others: brand colour, Apply without effect', () => {
    const { v, ctx } = setup({
      values: { ...RESET0, accent: '#3a7bd5' },
      brand: { userAccent: false, color: 'sage' },
    });
    expect(v.accentMore).toMatchObject({ hex: null, on: false, accent: 'sage' });
    v.accentMore.apply('#336699');
    expect(ctx.set).not.toHaveBeenCalled();
  });

  it('takes the own choice before the brand colour with userAccent', () => {
    const { v } = setup({ values: { ...RESET0, accent: 'rose' }, brand: { userAccent: true, color: 'sage' } });
    expect(v.accentOpts.filter((a) => a.on).map((a) => a.name)).toEqual(['rose']);
  });

  it('scrolls to the sections via the tabs', () => {
    const { v, ctx } = setup();
    expect(v.tabs.map((t) => t.label)).toEqual(['Color scheme', 'Accent color', 'Text & animation', 'Layout']);
    v.tabs.forEach((t) => t.onClick());
    expect(vi.mocked(ctx.scrollTo).mock.calls.flat()).toEqual(['mode', 'accent', 'text', 'layout']);
  });

  it('offers font size, density and animations and saves a choice', () => {
    const { v, ctx } = setup();
    expect(v.fontOpts.map((o) => [o.label, o.on])).toEqual([
      ['Small', false],
      ['Medium', true],
      ['Large', false],
    ]);
    expect(v.densityOpts.map((o) => [o.value, o.on])).toEqual([
      ['compact', false],
      ['normal', true],
      ['comfortable', false],
    ]);
    expect(v.tgAnim.on).toBe(true);
    v.fontOpts[0]!.onClick();
    v.densityOpts[2]!.onClick();
    v.tgAnim.onClick();
    expect(vi.mocked(ctx.set).mock.calls).toEqual([
      [{ font_size: 'small' }],
      [{ density: 'comfortable' }],
      [{ animations: false }],
    ]);
  });
});
