/**
 * Dragging sheets (mobile) and dialogs (tablet) down to close them: follows
 * the pointer, rubber band upwards, sliding out or springing back.
 *
 * The gesture paints directly on the DOM (offset of the panel, opacity of the
 * backdrop), not via Svelte state. One gesture at a time.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/swipe.ts`), trimmed to
 * dragging sheets and dialogs (no swiping of list rows and toasts, no
 * pulling up "More", no swiping back).
 */
import type { Attachment } from 'svelte/attachments';
import { hideTooltip } from './actions/tooltip.ts';

// --- Gesture maths (without DOM) ---

/** Distance in px from which a movement counts as a gesture (`gDecide` after 8 px). */
export const SLOP = 8;

/** Rubber band: grows with `d` but stays below `max` (`rubber`). */
export const rubber = (d: number, max: number): number => (max * d) / (d + max);

/** A sheet closes when dragged further than 30 % of its height and not flung back up, or when flung down faster than 0.5 px/ms. */
export const sheetCloses = (off: number, height: number, vy: number): boolean =>
  (off > height * 0.3 && vy > -0.3) || vy > 0.5;

/** Duration in ms of a sheet sliding out: its height at the release velocity (at least 1.6 px/ms), 150 to 300 ms. */
export const sheetOutMs = (height: number, vy: number): number =>
  Math.round(Math.max(150, Math.min(300, height / Math.max(Math.abs(vy), 1.6))));

/** Velocity in px/ms of a fling after `dt` ms: decays by 0.996 per ms. */
export const flingDecay = (v: number, dt: number): number => v * 0.996 ** dt;

/** Point of the pointer: x, y and time stamp of the event. */
export type TrackPoint = readonly [x: number, y: number, t: number];

/**
 * Velocity in px/ms over the last points, on release at time `at`; null if
 * the pointer was last standing still (90 ms without movement).
 */
export function velocity(points: readonly TrackPoint[], at: number): [number, number] {
  const a = points[0];
  const b = points[points.length - 1];
  if (!a || !b) return [0, 0];
  const dt = b[2] - a[2];
  if (dt <= 0 || at - b[2] >= 90) return [0, 0];
  return [(b[0] - a[0]) / dt, (b[1] - a[1]) / dt];
}

// --- Tracking the pointer ---

/** One pointer at a time, and no new gesture while one slides out. */
let tracking = false;
let animating = false;

export interface Steps {
  /** After 8 px: does the movement belong to this gesture? Otherwise tracking ends (scrolling stays with the browser). */
  decide: (dx: number, dy: number) => boolean;
  start: () => void;
  /** Movement since the point of decision, with the pointer event (at the decision the one that decided). */
  move: (dx: number, dy: number, e: PointerEvent) => void;
  /**
   * Released with the velocity in px/ms; `cancelled` if the browser cancelled
   * the gesture (`pointercancel`, e.g. a system gesture): then back without an
   * action.
   */
  end: (vx: number, vy: number, cancelled: boolean) => void;
}

const EDITABLE = 'input,textarea,select,[contenteditable="true"],[draggable="true"]';

/**
 * Gestures that wait for the decision on the pointer `down`: the attachments
 * of nested elements all join the same pointer, the innermost (first in the
 * bubbling order) is asked first and the first that takes the movement wins
 * (one `gDecide` for all gestures).
 */
let pending: { down: PointerEvent; candidates: Steps[]; drop: (steps: Steps) => void } | null = null;

/** May a gesture start at this place (`onGDown`)? Also while an inner gesture still waits for the decision on the same pointer. */
export function canStart(e: PointerEvent): boolean {
  if ((tracking && pending?.down !== e) || animating || !e.isPrimary || (e.pointerType === 'mouse' && e.button !== 0))
    return false;
  return !(e.target instanceof Element && e.target.closest(EDITABLE));
}

/**
 * Swallow the click after a gesture so that the row does not open and no
 * button triggers (`gSup`): the next click, for at most 300 ms – the browser
 * produces the click of a tap after `touchend`, not always in the same tick.
 */
function swallowClick(): void {
  const off = () => {
    clearTimeout(timer);
    window.removeEventListener('click', swallow, true);
  };
  const swallow = (e: Event) => {
    e.preventDefault();
    e.stopPropagation();
    off();
  };
  const timer = setTimeout(off, 300);
  window.addEventListener('click', swallow, true);
}

/**
 * Tracks the pointer from `down` across the window until release (`onGDown`,
 * `onGMove`, `onGUp`). A further call for the same `down` (an outer element)
 * only adds its steps as a candidate for the decision.
 *
 * Returns the cancellation for an element that goes away during the gesture:
 * its steps leave the decision, and if the gesture is its own (or nobody else
 * waits any more) the tracking ends without `end` and without swallowing the
 * next click.
 */
export function follow(down: PointerEvent, steps: Steps): () => void {
  if (tracking && pending?.down === down) {
    pending.candidates.push(steps);
    const { drop } = pending;
    return () => drop(steps);
  }
  tracking = true;
  const candidates = [steps];
  let done = false;
  const id = down.pointerId;
  let x0 = down.clientX;
  let y0 = down.clientY;
  let decided = false;
  const points: TrackPoint[] = [[down.clientX, down.clientY, down.timeStamp]];
  const root = document.documentElement.style;

  const stop = () => {
    done = true;
    tracking = false;
    pending = null;
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('pointercancel', onUp);
    root.userSelect = '';
    root.webkitUserSelect = '';
  };
  const onMove = (e: PointerEvent) => {
    if (e.pointerId !== id) return;
    points.push([e.clientX, e.clientY, e.timeStamp]);
    if (points.length > 6) points.shift();
    const dx = e.clientX - x0;
    const dy = e.clientY - y0;
    if (decided) {
      steps.move(dx, dy, e);
      return;
    }
    if (Math.hypot(dx, dy) < SLOP) return;
    const won = candidates.find((c) => c.decide(dx, dy));
    if (!won) {
      stop();
      return;
    }
    steps = won;
    pending = null;
    decided = true;
    x0 = e.clientX;
    y0 = e.clientY;
    // No text selection during the gesture (`gStart`).
    root.userSelect = 'none';
    root.webkitUserSelect = 'none';
    window.getSelection()?.removeAllRanges();
    hideTooltip();
    steps.start();
    steps.move(0, 0, e);
  };
  const onUp = (e: PointerEvent) => {
    if (e.pointerId !== id) return;
    stop();
    if (!decided) return;
    swallowClick();
    const cancelled = e.type !== 'pointerup';
    const [vx, vy] = cancelled ? [0, 0] : velocity(points, e.timeStamp);
    steps.end(vx, vy, cancelled);
  };
  const drop = (gone: Steps) => {
    if (done) return;
    if (decided) {
      if (steps === gone) stop();
      return;
    }
    const i = candidates.indexOf(gone);
    if (i >= 0) candidates.splice(i, 1);
    if (!candidates.length) stop();
  };
  pending = { down, candidates, drop };
  window.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);
  window.addEventListener('pointercancel', onUp);
  const own = steps;
  return () => drop(own);
}

/** Curves for springing back and sliding out; a sheet slides down with `SHEET_OUT`. */
const SPRING = 'cubic-bezier(.2,1.15,.35,1)';
const SHEET_OUT = 'cubic-bezier(.3,.6,.4,1)';

/** Runs `fn` after `ms` and blocks new gestures until then (`later`). */
function later(ms: number, fn: () => void): void {
  animating = true;
  setTimeout(() => {
    animating = false;
    fn();
  }, ms);
}

// --- Sheet ---

/** Momentum scrolling after releasing a sheet (`gFling`); a new touch stops it. */
let flingRaf = 0;

/** Scrolls `sc` on with the velocity `v` in px/ms until it stops or reaches the end. */
function fling(sc: HTMLElement, v: number): void {
  cancelAnimationFrame(flingRaf);
  if (Math.abs(v) < 0.05) return;
  let last = performance.now();
  const step = (now: number) => {
    const dt = Math.min(32, now - last);
    last = now;
    v = flingDecay(v, dt);
    const before = sc.scrollTop;
    sc.scrollTop = before + v * dt;
    if (Math.abs(v) > 0.03 && sc.scrollTop !== before) flingRaf = requestAnimationFrame(step);
  };
  flingRaf = requestAnimationFrame(step);
}

/** Nearest element between `from` and `stop` (exclusive) that scrolls vertically or horizontally (`gScroller`). */
function scroller(from: Element, stop: Element, axis: 'x' | 'y' = 'y'): HTMLElement | null {
  for (let n: Element | null = from; n && n !== stop; n = n.parentElement) {
    if (!(n instanceof HTMLElement)) continue;
    const cs = getComputedStyle(n);
    const o = axis === 'y' ? cs.overflowY : cs.overflowX;
    const more = axis === 'y' ? n.scrollHeight > n.clientHeight + 1 : n.scrollWidth > n.clientWidth + 1;
    if ((o === 'auto' || o === 'scroll') && more) return n;
  }
  return null;
}

/** Takes the gesture's inline styles off the panel and its backdrop again. */
function resetSheet(el: HTMLElement, scrim: HTMLElement | null): void {
  el.style.transition = '';
  el.style.translate = '';
  if (scrim) {
    scrim.style.transition = '';
    scrim.style.opacity = '';
  }
}

/** Closes the layer with a `mousedown` on its backdrop, exactly like a click on it. */
function closeVia(scrim: HTMLElement | null): void {
  scrim?.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }));
}

/**
 * Slides the panel down out of view in `ms` (to its height + 24 px) and fades
 * the backdrop; then `close` closes the layer. If the panel is still there
 * afterwards (the layer refused to close), it goes back into place.
 */
function slideOut(el: HTMLElement, scrim: HTMLElement | null, height: number, ms: number, close: () => void): void {
  el.style.transition = `translate ${ms}ms ${SHEET_OUT}`;
  el.style.translate = `0 ${height + 24}px`;
  if (scrim) {
    scrim.style.transition = `opacity ${ms}ms ease`;
    scrim.style.opacity = '0';
  }
  later(ms, () => {
    close();
    requestAnimationFrame(() => {
      if (el.isConnected) resetSheet(el, scrim);
    });
  });
}

/** Springs the panel back into place in `ms` and shows the backdrop fully; returns `ms`. */
function springIn(el: HTMLElement, scrim: HTMLElement | null, ms: number): number {
  el.style.transition = `translate ${ms}ms ${SPRING}`;
  el.style.translate = '0 0';
  if (scrim) {
    scrim.style.transition = `opacity ${ms}ms ease`;
    scrim.style.opacity = '1';
  }
  return ms;
}

/** Values of a sheet or dialog that can be dragged down to close it. */
export interface SheetDrag {
  /** Setting "Animations": without it, the sheet closes or comes back immediately. */
  animations: boolean;
  /**
   * Backdrop of the layer: fades while dragging, and the gesture closes the
   * layer with a `mousedown` on it, exactly like a click on the backdrop.
   * Without it the parent element (a dialog inside its backdrop).
   */
  scrim?: HTMLElement | null;
}

/**
 * Dragging a sheet (mobile) or dialog (tablet) down to close it, as an
 * attachment on the panel; `null` from `get` turns it off (desktop with a
 * mouse, a confirmation without "Cancel"). Vertical movement anywhere on the
 * panel except in fields; inside content that is scrolled, the content
 * scrolls first and the panel only follows from `scrollTop ≤ 0` downwards
 * (with a mouse it follows at once and scrolls the content itself).
 * Upwards only as a rubber band, the backdrop fades with the offset. Released
 * far or fast enough, the panel slides out and the layer closes; otherwise, and
 * always after `pointercancel`, it springs back. Content that was scrolled
 * keeps scrolling with momentum.
 */
export function dragSheet(get: () => SheetDrag | null): Attachment<HTMLElement> {
  return (el) => {
    let resetTimer: ReturnType<typeof setTimeout> | undefined;
    /** Touch that may become this gesture: where it went down, and whether the gesture has taken it. */
    let touch: { x: number; y: number; target: Element; taken: boolean } | null = null;
    /** Ends the tracking of the current pointer when the attachment goes away during the gesture. */
    let cancel: (() => void) | null = null;
    /** Takes the inline styles of a running drag off again (panel and backdrop), `null` without one. */
    let undo: (() => void) | null = null;

    /** Vertical movement the panel takes on touch: anywhere, inside scrolled content only downwards from its top. */
    const owns = (target: Element, dx: number, dy: number): boolean => {
      if (Math.abs(dx) > Math.abs(dy)) return false;
      const sc = scroller(target, el);
      return !sc || (dy > 0 && sc.scrollTop <= 0);
    };

    // The panel has `touch-action: pan-y` so that its content scrolls natively; the browser would then also
    // take a drag of the panel itself and cancel it (`pointercancel`). Its touch moves are kept from the
    // browser once the movement is the panel's drag (from `SLOP`, below the browsers' own pan thresholds of
    // about 10 to 15 px), so it follows the finger wherever it is held. Below `SLOP` nothing is cancelled:
    // a tap that drifts a few pixels keeps its click on the controls in the panel.
    const onTouchMove = (e: TouchEvent) => {
      const t = e.touches[0];
      if (!touch || !t || e.touches.length !== 1 || !e.cancelable) return;
      const dx = t.clientX - touch.x;
      const dy = t.clientY - touch.y;
      if (touch.taken || (Math.hypot(dx, dy) >= SLOP && owns(touch.target, dx, dy))) e.preventDefault();
    };
    const onTouchEnd = (e: TouchEvent) => {
      if (e.touches.length === 0) touch = null;
    };

    const onDown = (e: PointerEvent) => {
      cancelAnimationFrame(flingRaf);
      touch = null;
      if (!canStart(e)) return;
      const o = get();
      if (!o) return;
      const ms = (n: number) => (o.animations ? n : 0);
      const target = e.target instanceof Element ? e.target : el;
      const mouse = e.pointerType === 'mouse';
      let scrim: HTMLElement | null = null;
      let sc: HTMLElement | null = null;
      let st0 = 0;
      let height = 0;
      let off = 0;
      const own = e.pointerType === 'touch' ? { x: e.clientX, y: e.clientY, target, taken: false } : null;
      touch = own;

      cancel = follow(e, {
        decide: (dx, dy) => {
          if (Math.abs(dx) > Math.abs(dy) || (!mouse && !owns(target, dx, dy))) {
            // Not this gesture: the rest of the touch stays with the browser.
            if (touch === own) touch = null;
            return false;
          }
          sc = scroller(target, el);
          st0 = sc ? sc.scrollTop : 0;
          return true;
        },
        start: () => {
          if (own) own.taken = true;
          clearTimeout(resetTimer);
          scrim = o.scrim === undefined ? el.parentElement : o.scrim;
          const dragged = scrim;
          undo = () => resetSheet(el, dragged);
          height = el.offsetHeight;
          el.style.transition = 'none';
          if (scrim) scrim.style.transition = 'none';
        },
        move: (_dx, dy) => {
          off = dy;
          if (sc) {
            const want = st0 - dy;
            sc.scrollTop = Math.max(0, want);
            off = want < 0 ? -want : 0;
          }
          el.style.translate = `0 ${off >= 0 ? off : -rubber(-off, 30)}px`;
          if (scrim) scrim.style.opacity = String(Math.max(0, 1 - Math.max(0, off) / height));
        },
        end: (_vx, vy, cancelled) => {
          undo = null;
          if (sc && off <= 0) {
            if (!cancelled) fling(sc, -vy);
            resetSheet(el, scrim);
            return;
          }
          // Cancelled by the browser or the system (`pointercancel`): back, even beyond the threshold.
          if (!cancelled && sheetCloses(off, height, vy)) {
            slideOut(el, scrim, height, ms(sheetOutMs(height, vy)), () => closeVia(scrim));
            return;
          }
          resetTimer = setTimeout(() => resetSheet(el, scrim), springIn(el, scrim, ms(440)) + 30);
        },
      });
    };

    el.addEventListener('pointerdown', onDown);
    el.addEventListener('touchmove', onTouchMove, { passive: false });
    el.addEventListener('touchend', onTouchEnd);
    el.addEventListener('touchcancel', onTouchEnd);
    return () => {
      el.removeEventListener('pointerdown', onDown);
      el.removeEventListener('touchmove', onTouchMove);
      el.removeEventListener('touchend', onTouchEnd);
      el.removeEventListener('touchcancel', onTouchEnd);
      clearTimeout(resetTimer);
      // Removed during a gesture (the layer closed another way): stop following the pointer, without closing.
      cancel?.();
      undo?.();
    };
  };
}
