// Taken over from Roedy Mail (`apps/web/src/lib/components/Toast.test.ts`) with the toast as one prop and without
// the place above Mail's tab bar.
import { ACCENTS, derivedAccentVars, MODES, type CssVars, type ModeName } from '@roedy/tokens';
import { fireEvent, render } from '@testing-library/svelte';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Toast from './Toast.svelte';
import { TOAST_HOVER } from '../color.ts';
import { ui } from '../state/ui.svelte.ts';
import { contrast, hex, mixOklab, over, type Rgb } from '../testing/contrast.ts';
import { violations } from '../testing/axe.ts';

const SOURCE = readFileSync(join(import.meta.dirname, 'Toast.svelte'), 'utf8');

const swipe = { animations: false, hold: () => {}, away: () => {}, back: () => {} };

const notice = { id: 1, text: 'Saved', undo: false, error: false };
const withUndo = { id: 2, text: 'Device signed out', undo: true, error: false };

afterEach(() => {
  ui.toasts.discard();
});

// Colors of pill and "Undo" from the component's source (`SOURCE`)
function rule(selector: string): string {
  const escaped = selector.replace(/[\\^$.*+?()[\]{}|:]/g, '\\$&');
  const body = new RegExp(`^ {2}${escaped} \\{([^}]*)\\}`, 'm').exec(SOURCE)?.[1];
  if (body === undefined) throw new Error(`Rule ${selector} missing in Toast.svelte`);
  return body;
}

const varOf = (body: string, prop: string) => new RegExp(`${prop}: var\\((--[\\w-]+)\\)`).exec(body)?.[1];

/** Value of a variable for an accent: base value, `var(--acc)` or `color-mix(in oklab, var(--acc) P%, #rrggbb)`. */
function resolve(mode: ModeName, name: string, base: string): Rgb {
  const vars: CssVars = { ...MODES[mode], ...derivedAccentVars(mode), '--acc': 'var(--acc)' };
  const value = vars[name as `--${string}`] ?? '';
  if (value === 'var(--acc)') return hex(base);
  const mix = /^color-mix\(in oklab, var\(--acc\) (\d+)%, (#[0-9a-f]{6})\)$/.exec(value);
  if (mix) return mixOklab(hex(base), hex(mix[2]!), Number(mix[1]) / 100);
  if (/^#[0-9a-f]{6}$/.test(value)) return hex(value);
  throw new Error(`${name} unknown: ${value}`);
}

// WCAG AA 4.5:1 plus a margin for the approximation in `../testing/contrast.ts`
const MIN_CONTRAST = 4.6;

describe('toast', () => {
  it('announces a notice as status and an error as alert', () => {
    const ok = render(Toast, { toast: { id: 1, text: 'Saved', error: false }, isMobile: false });
    expect(ok.getByRole('status')).toHaveTextContent('Saved');
    ok.unmount();
    const err = render(Toast, { toast: { id: 2, text: 'Couldn’t save', error: true }, isMobile: true });
    expect(err.getByRole('alert')).toHaveTextContent('Couldn’t save');
  });

  it('shows the text and "Undo"', async () => {
    const onundo = vi.fn();
    const { getByRole } = render(Toast, { toast: withUndo, isMobile: false, onundo, swipe });
    expect(getByRole('status')).toHaveTextContent('Device signed out');
    await fireEvent.click(getByRole('button', { name: 'Undo' }));
    expect(onundo).toHaveBeenCalled();
  });

  it('undoes the action of `ui.toasts` without `onundo`', async () => {
    const revert = vi.fn();
    ui.toasts.defer({ text: 'Device signed out', revert });
    const { getByRole } = render(Toast, { toast: ui.toast!, isMobile: false });
    await fireEvent.click(getByRole('button', { name: 'Undo' }));
    expect(revert).toHaveBeenCalledOnce();
    expect(ui.toast).toBeNull();
  });

  it('stays in place in the desktop layout (`swipe` null) and "Undo" still works with the mouse', async () => {
    const onundo = vi.fn();
    const { getByRole } = render(Toast, { toast: withUndo, isMobile: false, onundo, swipe: null });
    const toast = getByRole('status');
    const at = (x: number, y: number) => ({
      clientX: x,
      clientY: y,
      pointerId: 1,
      isPrimary: true,
      pointerType: 'mouse',
    });
    await fireEvent.pointerDown(toast, at(100, 100));
    await fireEvent.pointerMove(window, at(100, 200));
    await fireEvent.pointerUp(window, at(100, 200));
    expect(toast.style.translate).toBe('');
    await fireEvent.click(getByRole('button', { name: 'Undo' }));
    expect(onundo).toHaveBeenCalled();
  });

  it('shows notices without undo, on mobile too', () => {
    const { getByRole, queryByRole } = render(Toast, { toast: notice, isMobile: true, swipe });
    expect(queryByRole('button')).toBeNull();
    expect(getByRole('status')).toHaveClass('mobile');
  });

  it('sits above the bar "Unsaved changes" while there is one and glides there', async () => {
    const view = render(Toast, { toast: notice, isMobile: false, lift: 106 });
    expect(view.getByRole('status')).toHaveInlineStyle({ bottom: '106px' });
    await view.rerender({ toast: notice, isMobile: false, lift: null });
    // Without the bar back to the position from the stylesheet.
    expect(view.getByRole('status')).toHaveInlineStyle({ bottom: '' });
    expect(rule('.toast')).toMatch(/transition: bottom var\(--dur, 220ms\) ease;/);
  });

  it('reports errors as `alert` with a warning icon and without undo', () => {
    const { getByRole, queryByRole, container } = render(Toast, {
      toast: { id: 3, text: 'Change not saved. Please try again.', undo: true, error: true },
      isMobile: false,
      swipe,
    });
    expect(getByRole('alert')).toHaveTextContent('Change not saved.');
    expect(queryByRole('button')).toBeNull();
    expect(container.querySelector('.tile.err')).not.toBeNull();
  });

  it('can be swiped (`data-g="toast"`)', () => {
    const { getByRole } = render(Toast, { toast: withUndo, isMobile: true, swipe });
    expect(getByRole('status')).toHaveAttribute('data-g', 'toast');
  });

  it('has no accessibility violations, with and without "Undo"', async () => {
    const view = render(Toast, { toast: withUndo, isMobile: false });
    expect(await violations(view)).toEqual([]);
    await view.rerender({ toast: notice, isMobile: false });
    expect(await violations(view)).toEqual([]);
  });

  describe('contrast of "Undo"', () => {
    const pill = varOf(rule('.toast'), 'background');
    const text = varOf(rule('.undo'), 'color');
    const hover = /background: color-mix\(in oklab, var\(--inkfg\) (\d+(?:\.\d+)?)%, transparent\)/.exec(
      rule('.undo:hover'),
    )?.[1];

    it('sits in `--toastacc` on `--ink`, hover from `--inkfg`', () => {
      expect(pill).toBe('--ink');
      expect(text).toBe('--toastacc');
      expect(hover).toBe('8');
      // Custom accents compute "Undo" for the same hover surface (`accentText()`).
      expect(Number(hover) / 100).toBe(TOAST_HOVER);
      expect(rule('.mobile .undo')).not.toMatch(/color:/);
    });

    for (const mode of ['light', 'dark'] as const) {
      it(`reaches ${MIN_CONTRAST}:1 in ${mode} mode with every accent color, also on hover`, () => {
        for (const [name, { base }] of Object.entries(ACCENTS)) {
          const bg = resolve(mode, pill!, base);
          const fg = resolve(mode, text!, base);
          // `color-mix(…, transparent)` is `--inkfg` with opacity over the pill
          const hovered = over(bg, hex(MODES[mode]['--inkfg']), Number(hover) / 100);
          expect(contrast(fg, bg), name).toBeGreaterThanOrEqual(MIN_CONTRAST);
          expect(contrast(fg, hovered), `${name} (Hover)`).toBeGreaterThanOrEqual(MIN_CONTRAST);
        }
      });
    }
  });
});
