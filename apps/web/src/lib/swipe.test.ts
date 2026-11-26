// Taken over from Roedy Mail (`apps/web/src/lib/swipe.test.ts`), trimmed to dragging sheets and dialogs.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { pointer } from '#lib/test/pointer.ts';
import { dragSheet, flingDecay, rubber, sheetCloses, sheetOutMs, velocity, type SheetDrag } from '#lib/swipe.ts';

describe('gesture maths', () => {
  it('rubber band', () => {
    expect(rubber(0, 28)).toBe(0);
    expect(rubber(1000, 28)).toBeLessThan(28);
  });

  it('sheet: closes from 30 % unless flung back up, or when flung down', () => {
    expect(sheetCloses(121, 400, 0)).toBe(true);
    expect(sheetCloses(120, 400, 0)).toBe(false);
    expect(sheetCloses(200, 400, -0.29)).toBe(true);
    expect(sheetCloses(200, 400, -0.3)).toBe(false);
    expect(sheetCloses(10, 400, 0.51)).toBe(true);
    expect(sheetCloses(10, 400, 0.5)).toBe(false);
  });

  it('sheet: slides out in 150 to 300 ms at the release velocity of at least 1.6 px/ms', () => {
    expect(sheetOutMs(400, 0)).toBe(250);
    expect(sheetOutMs(400, 2)).toBe(200);
    expect(sheetOutMs(400, 10)).toBe(150);
    expect(sheetOutMs(800, 0)).toBe(300);
    expect(sheetOutMs(800, -2)).toBe(300);
  });

  it('fling decays by 0.996 per ms', () => {
    expect(flingDecay(1, 0)).toBe(1);
    expect(flingDecay(1, 16)).toBeCloseTo(0.996 ** 16);
    expect(flingDecay(-2, 100)).toBeCloseTo(-2 * 0.996 ** 100);
  });

  it('velocity only with fresh movement', () => {
    const pts = [
      [0, 0, 0],
      [50, 10, 50],
    ] as const;
    expect(velocity(pts, 60)).toEqual([1, 0.2]);
    expect(velocity(pts, 140)).toEqual([0, 0]);
    expect(velocity([[0, 0, 0]], 10)).toEqual([0, 0]);
    expect(velocity([], 10)).toEqual([0, 0]);
  });
});

describe('dragSheet', () => {
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = '';
  });

  /** Dialog inside its backdrop (desktop layout), 400 px high, optionally with a scrolled list inside. */
  function sheet(o: SheetDrag | null = { animations: false }, scroll?: number) {
    const scrim = document.createElement('div');
    const el = document.createElement('div');
    el.innerHTML = '<div class="list" style="overflow-y: auto"><button type="button">Row</button></div>';
    scrim.append(el);
    document.body.append(scrim);
    Object.defineProperty(el, 'offsetHeight', { value: 400 });
    const list = el.querySelector<HTMLElement>('.list');
    if (!list) throw new Error('no list');
    if (scroll !== undefined) {
      Object.defineProperty(list, 'scrollHeight', { value: 1000 });
      Object.defineProperty(list, 'clientHeight', { value: 300 });
      list.scrollTop = scroll;
    }
    const closed = vi.fn();
    scrim.addEventListener('mousedown', (e) => {
      if (e.target === scrim) closed();
    });
    const off = dragSheet(() => o)(el);
    const row = list.querySelector('button');
    if (!row) throw new Error('no row');
    return { scrim, el, list, row, closed, off };
  }

  /** Drags from (100, 100) to `y`: decided after 10 px, released after standing still (no velocity). */
  function drag(target: Element, ...ys: number[]) {
    pointer('pointerdown', 100, 100, target);
    for (const y of ys) pointer('pointermove', 100, y);
    vi.advanceTimersByTime(200);
    pointer('pointerup', 100, ys[ys.length - 1] ?? 100);
    window.dispatchEvent(new MouseEvent('click'));
  }

  it('follows downwards, fades the backdrop and closes the layer via the backdrop when released beyond 30 %', () => {
    vi.useFakeTimers();
    const { scrim, el, closed } = sheet();
    pointer('pointerdown', 100, 100, el);
    pointer('pointermove', 100, 110);
    expect(el.style.transition).toBe('none');
    expect(scrim.style.transition).toBe('none');
    pointer('pointermove', 100, 270);
    expect(el.style.translate).toBe('0px 160px');
    expect(scrim.style.opacity).toBe('0.6');
    vi.advanceTimersByTime(200);
    pointer('pointerup', 100, 270);
    window.dispatchEvent(new MouseEvent('click'));
    expect(el.style.transition).toBe('translate 0ms cubic-bezier(0.3, 0.6, 0.4, 1)');
    expect(el.style.translate).toBe('0px 424px');
    expect(scrim.style.opacity).toBe('0');
    expect(closed).not.toHaveBeenCalled();
    vi.advanceTimersByTime(0);
    expect(closed).toHaveBeenCalledTimes(1);
    // The layer did not close (the test leaves it there): back into place.
    vi.advanceTimersByTime(20);
    expect(el.style.translate).toBe('');
    expect(scrim.style.opacity).toBe('');
  });

  it('slides out at the speed of the release', () => {
    vi.useFakeTimers();
    const { el, closed } = sheet({ animations: true });
    pointer('pointerdown', 100, 100, el);
    pointer('pointermove', 100, 110);
    vi.advanceTimersByTime(10);
    pointer('pointermove', 100, 140);
    vi.advanceTimersByTime(10);
    pointer('pointerup', 100, 160);
    window.dispatchEvent(new MouseEvent('click'));
    // Flung down at 4 px/ms: closes after only 50 px of 400 px, as fast as possible.
    expect(el.style.transition).toBe('translate 150ms cubic-bezier(0.3, 0.6, 0.4, 1)');
    vi.advanceTimersByTime(149);
    expect(closed).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(closed).toHaveBeenCalledTimes(1);
  });

  it('springs back in 440 ms when released before 30 %', () => {
    vi.useFakeTimers();
    const { scrim, el, closed } = sheet({ animations: true });
    drag(el, 110, 200);
    expect(el.style.transition).toBe('translate 440ms cubic-bezier(0.2, 1.15, 0.35, 1)');
    expect(el.style.translate).toBe('0px');
    expect(scrim.style.transition).toBe('opacity 440ms');
    expect(scrim.style.opacity).toBe('1');
    vi.advanceTimersByTime(470);
    expect(el.style.translate).toBe('');
    expect(el.style.transition).toBe('');
    expect(closed).not.toHaveBeenCalled();
  });

  it('springs back when the browser cancels the gesture, even beyond 30 %', () => {
    vi.useFakeTimers();
    const { el, closed } = sheet({ animations: true });
    pointer('pointerdown', 100, 100, el);
    pointer('pointermove', 100, 110);
    pointer('pointermove', 100, 300);
    expect(el.style.translate).toBe('0px 190px');
    pointer('pointercancel', 100, 300);
    window.dispatchEvent(new MouseEvent('click'));
    expect(el.style.transition).toBe('translate 440ms cubic-bezier(0.2, 1.15, 0.35, 1)');
    expect(el.style.translate).toBe('0px');
    vi.advanceTimersByTime(470);
    expect(closed).not.toHaveBeenCalled();
    expect(el.style.translate).toBe('');
  });

  it('goes upwards only as a rubber band', () => {
    vi.useFakeTimers();
    const { scrim, el } = sheet();
    pointer('pointerdown', 100, 100, el);
    pointer('pointermove', 100, 90);
    pointer('pointermove', 100, 30);
    expect(el.style.translate).toBe(`0px ${-rubber(60, 30)}px`);
    expect(scrim.style.opacity).toBe('1');
    vi.advanceTimersByTime(200);
    pointer('pointerup', 100, 30);
    window.dispatchEvent(new MouseEvent('click'));
    vi.advanceTimersByTime(30);
  });

  it('leaves scrolled content to the browser and follows from the top, scrolling the content again upwards', () => {
    vi.useFakeTimers();
    const scrolled = sheet({ animations: false }, 50);
    drag(scrolled.row, 110, 200);
    expect(scrolled.el.style.translate).toBe('');
    expect(scrolled.list.scrollTop).toBe(50);
    scrolled.off?.();
    document.body.innerHTML = '';

    const { el, list, row } = sheet({ animations: false }, 0);
    pointer('pointerdown', 100, 100, row);
    pointer('pointermove', 100, 110);
    pointer('pointermove', 100, 160);
    expect(el.style.translate).toBe('0px 50px');
    pointer('pointermove', 100, 70);
    expect(el.style.translate).toBe('0px');
    expect(list.scrollTop).toBe(40);
    vi.advanceTimersByTime(200);
    pointer('pointerup', 100, 70);
    window.dispatchEvent(new MouseEvent('click'));
    // Released while the content scrolls: the sheet stays.
    expect(el.style.translate).toBe('');
  });

  it('ignores sideways movement, fields and a missing gesture', () => {
    vi.useFakeTimers();
    const { el } = sheet();
    pointer('pointerdown', 100, 100, el);
    pointer('pointermove', 120, 100);
    pointer('pointermove', 160, 140);
    expect(el.style.translate).toBe('');
    pointer('pointerup', 160, 140);

    el.insertAdjacentHTML('beforeend', '<input>');
    const input = el.querySelector('input');
    if (!input) throw new Error('no input');
    drag(input, 110, 300);
    expect(el.style.translate).toBe('');
    document.body.innerHTML = '';

    const off = sheet(null);
    drag(off.el, 110, 300);
    expect(off.el.style.translate).toBe('');
    expect(off.closed).not.toHaveBeenCalled();
  });

  /** Touch event at (x, y) on `target`; returns whether the gesture kept it from the browser. */
  function touch(type: string, x: number, y: number, target: Element): boolean {
    const e = new Event(type, { bubbles: true, cancelable: true });
    const touches = type === 'touchmove' ? [{ clientX: x, clientY: y }] : [];
    Object.assign(e, { touches });
    target.dispatchEvent(e);
    return e.defaultPrevented;
  }

  it('keeps a vertical touch on the panel from the browser, so it follows wherever it is held', () => {
    vi.useFakeTimers();
    const { el, row } = sheet({ animations: false });
    pointer('pointerdown', 100, 100, row);
    // Below the slop (8 px) a tap may drift: nothing is cancelled, so its click survives.
    expect(touch('touchmove', 100, 104, row)).toBe(false);
    // From the slop on, the movement is the panel's, before the browser would start its own pan.
    expect(touch('touchmove', 100, 109, row)).toBe(true);
    pointer('pointermove', 100, 110);
    expect(touch('touchmove', 100, 110, row)).toBe(true);
    pointer('pointermove', 100, 160);
    // Upwards again: still the panel's movement.
    expect(touch('touchmove', 100, 60, row)).toBe(true);
    expect(el.style.translate).not.toBe('');
    vi.advanceTimersByTime(200);
    pointer('pointerup', 100, 60);
    window.dispatchEvent(new MouseEvent('click'));
    touch('touchend', 100, 60, row);
    // A touch that the gesture did not start on is left alone.
    expect(touch('touchmove', 100, 200, row)).toBe(false);
  });

  it('leaves horizontal touches and scrolled content to the browser', () => {
    vi.useFakeTimers();
    const flat = sheet({ animations: false });
    pointer('pointerdown', 100, 100, flat.row);
    expect(touch('touchmove', 106, 101, flat.row)).toBe(false);
    pointer('pointermove', 112, 102);
    // Decided as no gesture: the rest of the touch stays with the browser, also vertically.
    expect(touch('touchmove', 112, 150, flat.row)).toBe(false);
    pointer('pointerup', 112, 150);
    touch('touchend', 112, 150, flat.row);
    flat.off?.();
    document.body.innerHTML = '';

    const { row } = sheet({ animations: false }, 50);
    pointer('pointerdown', 100, 100, row);
    // Beyond the slop too: the content scrolls (down while not at its top, and up).
    expect(touch('touchmove', 100, 110, row)).toBe(false);
    expect(touch('touchmove', 100, 90, row)).toBe(false);
    pointer('pointercancel', 100, 94);
    touch('touchcancel', 100, 94, row);
  });
});
