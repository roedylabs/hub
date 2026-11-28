// Taken over from Roedy Mail (`apps/web/src/lib/components/TimePicker.test.ts`) with `ui.animations` instead of
// Mail's preferences, plus the German texts and axe.
import { cleanup, fireEvent, render, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { connectSettings } from '../settings.svelte.ts';
import { ui } from '../state/ui.svelte.ts';
import { violations } from '../testing/axe.ts';
import { timePickValues, type TimePickContext, type TimePickInput } from '../time-picker.ts';
import TimePicker from './TimePicker.svelte';

function context() {
  return {
    set: vi.fn<TimePickContext['set']>(),
    show: vi.fn<TimePickContext['show']>(),
    edit: vi.fn<TimePickContext['edit']>(),
  } satisfies TimePickContext;
}

function values(over: Partial<TimePickInput> = {}, ctx: TimePickContext = context()) {
  return timePickValues(
    {
      title: 'Quiet hours end',
      value: '07:30',
      disabled: false,
      h12: false,
      open: true,
      edit: null,
      isMobile: false,
      ...over,
    },
    ctx,
  );
}

// jsdom does not know `scrollTo` on elements.
const scrollTo = vi.fn();

beforeEach(() => {
  HTMLElement.prototype.scrollTo = scrollTo;
});

afterEach(() => {
  scrollTo.mockClear();
  ui.animations = true;
});

/** Cells of a desktop column: a pointer aid, hidden from screen readers (the field takes the keyboard). */
function cells(view: { container: HTMLElement }, col: 'h' | 'm'): HTMLButtonElement[] {
  return [...view.container.querySelectorAll<HTMLButtonElement>(`[data-tcol="${col}"] button`)];
}

describe('time picker', () => {
  it('shows hours and minutes with the chosen time on desktop', () => {
    const view = render(TimePicker, { timePick: values(), isMobile: false });
    expect(view.getByText('Hour')).toBeInTheDocument();
    expect(view.getByText('Minute')).toBeInTheDocument();
    expect(view.queryByRole('button', { name: 'Done' })).toBeNull();
    expect(view.queryByText('Quiet hours end')).toBeNull();
    // No listboxes on desktop: the columns are hidden from screen readers and out of the tab order.
    expect(view.queryByRole('listbox')).toBeNull();
    expect(view.container.querySelector('[data-tcol="h"]')).toHaveAttribute('aria-hidden', 'true');
    expect(view.container.querySelector('[data-tcol="m"]')).toHaveAttribute('aria-hidden', 'true');
    const h = cells(view, 'h');
    expect(h).toHaveLength(24);
    expect(h[0]).toHaveTextContent('00');
    expect(h[0]).toHaveAttribute('tabindex', '-1');
    expect(h[0]).not.toHaveAttribute('role');
    expect(cells(view, 'm')).toHaveLength(60);
    const selH = cells(view, 'h').find((c) => c.classList.contains('on'));
    expect(selH).toHaveTextContent('07');
    expect(selH).toHaveStyle({ fontWeight: '800' });
    expect(cells(view, 'm').find((c) => c.classList.contains('on'))).toHaveTextContent('30');
    expect(h[1]).toHaveStyle({ fontWeight: '600' });
    expect(h[1]).not.toHaveClass('on');
  });

  it('chooses hour and minute', async () => {
    const ctx = context();
    const view = render(TimePicker, { timePick: values({}, ctx), isMobile: false });
    await fireEvent.click(cells(view, 'h')[22]!);
    expect(ctx.set).toHaveBeenLastCalledWith('22:30');
    expect(ctx.show).not.toHaveBeenCalled();
    await fireEvent.click(cells(view, 'm')[45]!);
    expect(ctx.set).toHaveBeenLastCalledWith('07:45');
    expect(ctx.show).toHaveBeenLastCalledWith(false);
  });

  it('shows hours with AM/PM in 12-hour format', () => {
    const view = render(TimePicker, { timePick: values({ h12: true, value: '22:00' }), isMobile: false });
    expect(cells(view, 'h')[0]).toHaveTextContent('12 AM');
    expect(cells(view, 'h').find((c) => c.classList.contains('on'))).toHaveTextContent('10 PM');
  });

  it('shows title, close and "Done" on mobile', async () => {
    const ctx = context();
    const view = render(TimePicker, { timePick: values({ isMobile: true }, ctx), isMobile: true });
    expect(view.container.querySelector('.time-pick')).toHaveClass('mobile');
    expect(view.getByText('Quiet hours end')).toBeInTheDocument();
    expect(view.queryByText('Hour')).toBeNull();
    const hours = view.getByRole('listbox', { name: 'Hour' });
    expect(within(hours).getAllByRole('option')[0]).not.toHaveAttribute('tabindex');
    // The minute does not close on mobile.
    await fireEvent.click(within(view.getByRole('listbox', { name: 'Minute' })).getByRole('option', { name: '15' }));
    expect(ctx.set).toHaveBeenLastCalledWith('07:15');
    expect(ctx.show).not.toHaveBeenCalled();
    await fireEvent.click(view.getByRole('button', { name: 'Close' }));
    expect(ctx.show).toHaveBeenLastCalledWith(false);
    ctx.show.mockClear();
    await fireEvent.click(view.getByRole('button', { name: 'Done' }));
    expect(ctx.show).toHaveBeenCalledWith(false);
  });

  it('aligns the columns immediately on open, smoothly afterwards', async () => {
    const view = render(TimePicker, { timePick: values(), isMobile: false });
    expect(scrollTo).not.toHaveBeenCalled();
    await view.rerender({ timePick: values({ value: '08:30' }), isMobile: false });
    expect(scrollTo).toHaveBeenCalledWith(expect.objectContaining({ behavior: 'smooth' }));
    scrollTo.mockClear();
    ui.animations = false;
    await view.rerender({ timePick: values({ value: '09:30' }), isMobile: false });
    expect(scrollTo).not.toHaveBeenCalled();
  });

  it('has its German texts', () => {
    connectSettings({ lang: () => 'de' });
    try {
      const view = render(TimePicker, { timePick: values({ isMobile: true }), isMobile: true });
      expect(view.getByRole('listbox', { name: 'Stunde' })).toBeInTheDocument();
      expect(view.getByRole('button', { name: 'Schließen' })).toBeInTheDocument();
      expect(view.getByRole('button', { name: 'Fertig' })).toBeInTheDocument();
    } finally {
      connectSettings({});
    }
  });

  it('has no axe violations on desktop and mobile', async () => {
    expect(await violations(render(TimePicker, { timePick: values(), isMobile: false }))).toEqual([]);
    cleanup();
    expect(await violations(render(TimePicker, { timePick: values({ isMobile: true }), isMobile: true }))).toEqual([]);
  });
});
