import { describe, expect, it, vi } from 'vitest';
import { generalValues, type GeneralContext } from '#lib/general.ts';
import { RESET0 } from '#lib/state/prefs.svelte.ts';

function setup(values = RESET0) {
  const ctx: GeneralContext = { set: vi.fn(), scrollTo: vi.fn() };
  return { ctx, v: generalValues({ values }, ctx) };
}

describe('generalValues', () => {
  it('shows the defaults', () => {
    const { v } = setup();
    expect([v.tgConfirm.on, v.tgKeys.on, v.tgDeveloper.on]).toEqual([true, true, false]);
    expect(v.undoOpts.map((o) => [o.label, o.on])).toEqual([
      ['3 s', false],
      ['5 s', true],
      ['10 s', false],
    ]);
  });

  it('saves each change at once', () => {
    const { v, ctx } = setup({ ...RESET0, developer: true });
    v.tgConfirm.onClick();
    v.undoOpts[2]!.onClick();
    v.tgKeys.onClick();
    v.tgDeveloper.onClick();
    expect(vi.mocked(ctx.set).mock.calls).toEqual([
      [{ confirm_delete: false }],
      [{ undo_delay: 10 }],
      [{ shortcuts: false }],
      [{ developer: false }],
    ]);
  });

  it('scrolls to the sections via the tabs', () => {
    const { v, ctx } = setup();
    v.genTabs.forEach((t) => t.onClick());
    expect(vi.mocked(ctx.scrollTo).mock.calls.flat()).toEqual(['behavior', 'developer']);
  });
});
