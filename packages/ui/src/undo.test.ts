// Taken over from Roedy Mail (`apps/web/src/lib/undo.test.ts`) with the options instead of `gUndo` and the
// connection from `network`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { network } from './state/network.svelte.ts';
import { ERROR_TOAST_MS, TOAST_MS, Toasts, UNDO_DELAYS, UNDO_MS, type ToastItem } from './undo.ts';

let shown: (ToastItem | null)[];
let toasts: Toasts;

beforeEach(() => {
  vi.useFakeTimers();
  shown = [];
  toasts = new Toasts((t) => shown.push(t), { undoMs: () => 5000 });
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  network.online = true;
  network.reached = new Map();
});

const last = () => shown.at(-1) ?? null;

/** Action with mocks for reverting and sending. */
function action(commit = vi.fn(() => Promise.resolve())) {
  return { text: 'Device signed out', revert: vi.fn(), commit, onError: vi.fn() };
}

describe('Toasts', () => {
  it('shows notices without undo for 3.2 s', () => {
    toasts.show('Calendar coming soon');
    expect(last()).toMatchObject({ text: 'Calendar coming soon', undo: false });
    expect(TOAST_MS).toBe(3200);
    vi.advanceTimersByTime(TOAST_MS - 1);
    expect(last()).not.toBeNull();
    vi.advanceTimersByTime(1);
    expect(last()).toBeNull();
  });

  it('shows errors as an error toast without undo for 4.6 s', async () => {
    toasts.show('Notice');
    expect(last()).toMatchObject({ error: false });
    const a = action();
    toasts.defer(a);
    expect(last()).toMatchObject({ undo: true, error: false });
    toasts.error('That did not work. Please try again.');
    expect(last()).toMatchObject({
      text: 'That did not work. Please try again.',
      undo: false,
      error: true,
    });
    // A pending action goes out first; "Undo" reverts nothing afterwards.
    await vi.advanceTimersByTimeAsync(0);
    expect(a.commit).toHaveBeenCalledWith(false);
    toasts.undo();
    expect(a.revert).not.toHaveBeenCalled();
    toasts.error('Copying not possible.');
    expect(ERROR_TOAST_MS).toBe(4600);
    vi.advanceTimersByTime(ERROR_TOAST_MS - 1);
    expect(last()).toMatchObject({ text: 'Copying not possible.', error: true });
    vi.advanceTimersByTime(1);
    expect(last()).toBeNull();
  });

  it('sends an action only after the toast expires', async () => {
    const a = action();
    toasts.defer(a);
    expect(last()).toMatchObject({ text: 'Device signed out', undo: true });
    vi.advanceTimersByTime(4999);
    expect(a.commit).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(a.commit).toHaveBeenCalledWith(false);
    expect(last()).toBeNull();
    expect(a.revert).not.toHaveBeenCalled();
  });

  it('knows whether an action is still waiting in the toast', async () => {
    const a = action();
    const b = action();
    expect(toasts.waiting(null)).toBe(false);
    toasts.defer(a);
    expect([toasts.waiting(a), toasts.waiting(b)]).toEqual([true, false]);
    toasts.defer(b);
    expect([toasts.waiting(a), toasts.waiting(b)]).toEqual([false, true]);
    toasts.undo();
    expect(toasts.waiting(b)).toBe(false);
    toasts.defer(a);
    await vi.advanceTimersByTimeAsync(5000);
    expect(toasts.waiting(a)).toBe(false);
  });

  it('hides notices when hiding and sends a pending action', async () => {
    toasts.show('Redirect URI copied');
    toasts.dismiss();
    expect(last()).toBeNull();
    const a = action();
    toasts.defer(a);
    toasts.dismiss();
    expect(last()).toBeNull();
    await vi.advanceTimersByTimeAsync(0);
    expect(a.commit).toHaveBeenCalledWith(false);
    expect(a.revert).not.toHaveBeenCalled();
  });

  it('pauses while swiping and runs for the given time after springing back', async () => {
    const a = action();
    toasts.defer(a);
    toasts.hold();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(last()?.text).toBe('Device signed out');
    expect(a.commit).not.toHaveBeenCalled();
    toasts.resume(2400);
    await vi.advanceTimersByTimeAsync(2399);
    expect(a.commit).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(last()).toBeNull();
    expect(a.commit).toHaveBeenCalledWith(false);
  });

  it('pauses an error toast while swiping and lets it run 2.4 s after springing back', () => {
    toasts.error('Copying not possible.');
    vi.advanceTimersByTime(ERROR_TOAST_MS - 1);
    toasts.hold();
    vi.advanceTimersByTime(60_000);
    expect(last()).toMatchObject({ text: 'Copying not possible.', error: true });
    toasts.resume(2400);
    vi.advanceTimersByTime(2399);
    expect(last()).toMatchObject({ error: true });
    vi.advanceTimersByTime(1);
    expect(last()).toBeNull();
  });

  it('does not restart without a paused toast', async () => {
    toasts.resume(2400);
    expect(shown).toEqual([]);
    toasts.show('Calendar coming soon');
    toasts.hold();
    toasts.dismiss();
    toasts.resume(2400);
    await vi.advanceTimersByTimeAsync(5000);
    expect(shown.filter((t) => t === null)).toHaveLength(1);
  });

  it('reverts only the local change with "Undo"', async () => {
    const a = action();
    toasts.defer(a);
    toasts.undo();
    expect(a.revert).toHaveBeenCalled();
    expect(last()).toBeNull();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(a.commit).not.toHaveBeenCalled();
  });

  it('compensates on the server only after "Undo", not when discarding', () => {
    const onUndo = vi.fn();
    const a = { ...action(), onUndo };
    toasts.defer(a);
    expect(toasts.discard()).toBe(a);
    expect(a.revert).toHaveBeenCalled();
    expect(onUndo).not.toHaveBeenCalled();
    const b = { ...action(), onUndo };
    toasts.defer(b);
    toasts.undo();
    expect(b.revert).toHaveBeenCalled();
    expect(onUndo).toHaveBeenCalledTimes(1);
    expect(toasts.discard()).toBeNull();
  });

  it('sends the previous action immediately when a new toast comes', async () => {
    const first = action();
    const second = action();
    toasts.defer(first);
    toasts.defer(second);
    await vi.advanceTimersByTimeAsync(0);
    expect(first.commit).toHaveBeenCalledWith(false);
    expect(second.commit).not.toHaveBeenCalled();
    // Undo now only applies to the second action.
    toasts.undo();
    expect(first.revert).not.toHaveBeenCalled();
    expect(second.revert).toHaveBeenCalled();
    toasts.defer(action());
    toasts.show('Notice');
    expect(last()).toMatchObject({ text: 'Notice', undo: false });
  });

  it('reverts the change and reports when sending fails', async () => {
    const error = new Error('offline');
    const a = action(vi.fn(() => Promise.reject(error)));
    toasts.defer(a);
    await vi.advanceTimersByTimeAsync(5000);
    expect(a.revert).toHaveBeenCalled();
    expect(a.onError).toHaveBeenCalledWith(error);
  });

  it('sends immediately with keepalive when leaving and hiding the page', async () => {
    const detach = toasts.attach(window);
    for (const fire of [
      () => window.dispatchEvent(new Event('pagehide')),
      () => {
        vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
        document.dispatchEvent(new Event('visibilitychange'));
      },
    ]) {
      const a = action();
      toasts.defer(a);
      fire();
      await vi.advanceTimersByTimeAsync(0);
      expect(a.commit).toHaveBeenCalledWith(true);
      expect(last()).toBeNull();
    }
    detach();
    const a = action();
    toasts.defer(a);
    window.dispatchEvent(new Event('pagehide'));
    expect(a.commit).not.toHaveBeenCalled();
  });

  it('does not send early without a connection but waits for the expiry', async () => {
    const detach = toasts.attach(window);
    const a = action();
    toasts.defer(a);
    window.dispatchEvent(new Event('offline'));
    await vi.advanceTimersByTimeAsync(0);
    expect(a.commit).not.toHaveBeenCalled();
    expect(last()).toMatchObject({ undo: true });

    // Offline according to the requests: leaving or hiding the page sends nothing early either.
    network.record('https://app.example.test', false);
    expect(network.offline).toBe(true);
    window.dispatchEvent(new Event('pagehide'));
    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(0);
    expect(a.commit).not.toHaveBeenCalled();
    expect(last()).toMatchObject({ undo: true });
    await vi.advanceTimersByTimeAsync(5000);
    expect(a.commit).toHaveBeenCalledWith(false);
    detach();
  });

  it('asks the given connection instead of `network`', async () => {
    let offline = true;
    const own = new Toasts((t) => shown.push(t), { offline: () => offline });
    const detach = own.attach(window);
    const a = action();
    own.defer(a);
    window.dispatchEvent(new Event('pagehide'));
    await vi.advanceTimersByTimeAsync(0);
    expect(a.commit).not.toHaveBeenCalled();
    offline = false;
    window.dispatchEvent(new Event('pagehide'));
    await vi.advanceTimersByTimeAsync(0);
    expect(a.commit).toHaveBeenCalledWith(true);
    detach();
  });

  it('shows a toast with undo for 5 s without an undo delay', async () => {
    expect(UNDO_DELAYS).toContain(UNDO_MS);
    const own = new Toasts((t) => shown.push(t));
    const a = action();
    own.defer(a);
    await vi.advanceTimersByTimeAsync(UNDO_MS - 1);
    expect(a.commit).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(a.commit).toHaveBeenCalledWith(false);
  });

  it('reads the undo delay per toast', () => {
    let ms = 3000;
    const own = new Toasts((t) => shown.push(t), { undoMs: () => ms });
    own.defer(action());
    vi.advanceTimersByTime(3000);
    expect(last()).toBeNull();
    ms = 10_000;
    own.defer(action());
    vi.advanceTimersByTime(9999);
    expect(last()).toMatchObject({ undo: true });
    vi.advanceTimersByTime(1);
    expect(last()).toBeNull();
  });

  it('waits when flushing until the action is sent (signing out)', async () => {
    let done!: () => void;
    const a = action(vi.fn(() => new Promise<void>((resolve) => (done = resolve))));
    toasts.defer(a);
    let flushed = false;
    const flushing = toasts.flush().then(() => (flushed = true));
    await vi.advanceTimersByTimeAsync(0);
    expect(flushed).toBe(false);
    done();
    await flushing;
    expect(flushed).toBe(true);
  });

  it('also waits when flushing for actions that are already on their way', async () => {
    let done!: () => void;
    const a = action(vi.fn(() => new Promise<void>((resolve) => (done = resolve))));
    toasts.defer(a);
    // The toast expires, sending starts.
    await vi.advanceTimersByTimeAsync(5000);
    expect(a.commit).toHaveBeenCalled();
    let flushed = false;
    const flushing = toasts.flush().then(() => (flushed = true));
    await vi.advanceTimersByTimeAsync(0);
    expect(flushed).toBe(false);
    done();
    await flushing;
    expect(flushed).toBe(true);
  });

  describe('Undo as a call to the server', () => {
    function offer(undo = vi.fn(() => Promise.resolve())) {
      return { text: 'Message sent', undo, onUndone: vi.fn(), onError: vi.fn() };
    }

    it('calls the server on "Undo" and reports the result', async () => {
      const a = offer();
      toasts.undoable(a);
      expect(last()).toMatchObject({ text: 'Message sent', undo: true, error: false });
      toasts.undo();
      expect(last()).toBeNull();
      expect(a.undo).toHaveBeenCalledOnce();
      await vi.advanceTimersByTimeAsync(0);
      expect(a.onUndone).toHaveBeenCalledOnce();
      expect(a.onError).not.toHaveBeenCalled();
    });

    it('reports a failed revert', async () => {
      const error = new Error('cannotUnsend');
      const a = offer(vi.fn(() => Promise.reject(error)));
      toasts.undoable(a);
      toasts.undo();
      await vi.advanceTimersByTimeAsync(0);
      expect(a.onError).toHaveBeenCalledWith(error);
      expect(a.onUndone).not.toHaveBeenCalled();
    });

    it('stays when the page is hidden and runs for at most `ms`', async () => {
      const a = { ...offer(), ms: 2000 };
      toasts.undoable(a);
      await toasts.flush(true);
      expect(last()).toMatchObject({ undo: true });
      toasts.undo();
      await vi.advanceTimersByTimeAsync(0);
      expect(a.undo).toHaveBeenCalledOnce();

      const b = { ...offer(), ms: 2000 };
      toasts.undoable(b);
      vi.advanceTimersByTime(2000);
      expect(last()).toBeNull();
    });

    it('never lets a swipe extend the undo beyond its deadline', async () => {
      const a = { ...offer(), ms: 2000 };
      toasts.undoable(a);
      vi.advanceTimersByTime(1500);
      toasts.hold();
      vi.advanceTimersByTime(1000);
      // Past the deadline while held: "Undo" no longer calls the server, the toast goes.
      toasts.undo();
      expect(a.undo).not.toHaveBeenCalled();
      expect(last()).toBeNull();

      const b = { ...offer(), ms: 2000 };
      toasts.undoable(b);
      vi.advanceTimersByTime(1500);
      toasts.hold();
      vi.advanceTimersByTime(300);
      // Sprung back: runs only for the 200 ms left, not another 2400.
      toasts.resume(2400);
      vi.advanceTimersByTime(199);
      expect(last()).toMatchObject({ undo: true });
      vi.advanceTimersByTime(1);
      expect(last()).toBeNull();

      const c = { ...offer(), ms: 2000 };
      toasts.undoable(c);
      toasts.hold();
      vi.advanceTimersByTime(2500);
      toasts.resume(2400);
      expect(last()).toBeNull();
      await vi.advanceTimersByTimeAsync(0);
      expect(c.undo).not.toHaveBeenCalled();
    });

    it('an error after reverting does not hold up signing out', async () => {
      const a = {
        ...offer(),
        onUndone: vi.fn(() => {
          throw new Error('Navigation');
        }),
      };
      toasts.undoable(a);
      toasts.undo();
      await expect(toasts.flush()).resolves.toBeUndefined();
    });

    it('after expiry, a new toast or signing out nothing happens any more', async () => {
      for (const end of [
        () => vi.advanceTimersByTime(5000),
        () => toasts.show('Other message'),
        () => toasts.discard(),
      ]) {
        const a = offer();
        toasts.undoable(a);
        end();
        toasts.undo();
        await vi.advanceTimersByTimeAsync(0);
        expect(a.undo).not.toHaveBeenCalled();
      }
    });

    it('sends a pending action first; signing out waits for the revert', async () => {
      const deferred = action();
      toasts.defer(deferred);
      let done!: () => void;
      const a = offer(vi.fn(() => new Promise<void>((r) => (done = r))));
      toasts.undoable(a);
      expect(deferred.commit).toHaveBeenCalledOnce();
      toasts.undo();
      let flushed = false;
      const flushing = toasts.flush().then(() => (flushed = true));
      await vi.advanceTimersByTimeAsync(0);
      expect(flushed).toBe(false);
      done();
      await flushing;
      expect(a.onUndone).toHaveBeenCalledOnce();
    });
  });
});
