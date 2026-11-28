import { flushSync } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { formatDate, formatNumber, formatTime } from './date.ts';
import { connectHub, hubLang, hubPrefs } from './hub.svelte.ts';
import { DEFAULT_PREFS, PREFS_KEY } from './prefs.ts';
import { settings } from './settings.svelte.ts';
import { ui } from './state/ui.svelte.ts';

let disconnect: (() => void) | undefined;

afterEach(() => {
  disconnect?.();
  disconnect = undefined;
  localStorage.clear();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function stubSession(prefs: unknown, status = 200): ReturnType<typeof vi.fn> {
  const fetch = vi.fn(() =>
    Promise.resolve(new Response(JSON.stringify(status === 200 ? { prefs } : { error: 'unauthorized' }), { status })),
  );
  vi.stubGlobal('fetch', fetch);
  return fetch;
}

const t = Date.parse('2026-10-25T14:57:00Z');

describe('connectHub', () => {
  it('applies the copy at once, then the session', async () => {
    localStorage.setItem(PREFS_KEY, JSON.stringify({ font_size: 'large', language: 'de', theme: 'dark' }));
    let answer: (value: Response) => void = () => {};
    vi.stubGlobal(
      'fetch',
      vi.fn(() => new Promise<Response>((resolve) => (answer = resolve))),
    );
    const root = document.createElement('html');
    disconnect = connectHub({ root });
    flushSync();
    expect(root.dataset.fontSize).toBe('large');
    expect(settings().lang()).toBe('de');
    expect(settings().mode()).toBe('dark');
    expect(hubPrefs().density).toBe(DEFAULT_PREFS.density);

    answer(
      new Response(
        JSON.stringify({
          prefs: {
            ...DEFAULT_PREFS,
            density: 'compact',
            animations: false,
            time_zone: 'Asia/Tokyo',
            time_zone_auto: false,
            date_format: 'us',
            time_format: '12h',
            number_format: '1.234,56',
            undo_delay: 10,
            confirm_delete: false,
            shortcuts: false,
            developer: true,
          },
        }),
      ),
    );
    await vi.waitFor(() => expect(hubPrefs().density).toBe('compact'));
    flushSync();
    expect(root.dataset).toMatchObject({ fontSize: 'medium', density: 'compact', animations: 'off' });
    expect(settings().lang()).toBe('en');
    expect(settings().timeZone()).toBe('Asia/Tokyo');
    expect(formatDate(t)).toBe('10/25/2026');
    expect(formatTime(t)).toBe('11:57 PM');
    expect(formatNumber(1234.5, { minimumFractionDigits: 2 })).toBe('1.234,50');
    expect(ui.undoMs).toBe(10_000);
    expect(ui.motion).toBe(false);
    expect([ui.confirmDelete, ui.shortcuts, ui.developer]).toEqual([false, false, true]);
  });

  it('follows changes in another tab', () => {
    stubSession(null, 401);
    disconnect = connectHub({ root: null });
    expect(hubPrefs().week_start).toBe('monday');
    localStorage.setItem(PREFS_KEY, JSON.stringify({ week_start: 'sunday' }));
    window.dispatchEvent(new StorageEvent('storage', { key: PREFS_KEY }));
    expect(hubPrefs().week_start).toBe('sunday');
    expect(settings().weekStart()).toBe('sunday');
    // Other keys are not the hub's settings.
    localStorage.setItem(PREFS_KEY, JSON.stringify({ week_start: 'monday' }));
    window.dispatchEvent(new StorageEvent('storage', { key: 'roedy.mail.x' }));
    expect(hubPrefs().week_start).toBe('sunday');
  });

  it('keeps the copy when signed out and drops what it does not know', async () => {
    localStorage.setItem(PREFS_KEY, JSON.stringify({ undo_delay: 3 }));
    const fetch = stubSession(null, 401);
    disconnect = connectHub({ root: null });
    await vi.waitFor(() => expect(fetch).toHaveBeenCalled());
    await Promise.resolve();
    expect(hubPrefs().undo_delay).toBe(3);

    disconnect();
    stubSession({ undo_delay: 4, region: 'XX', density: 'comfortable' });
    disconnect = connectHub({ root: null });
    await vi.waitFor(() => expect(hubPrefs().density).toBe('comfortable'));
    expect(hubPrefs().undo_delay).toBe(DEFAULT_PREFS.undo_delay);
    expect(hubPrefs().region).toBeNull();
  });

  it('can take a session the app already loaded', () => {
    const fetch = stubSession({});
    disconnect = connectHub({ root: null, session: { prefs: { region: 'AT' } } });
    return vi.waitFor(() => {
      expect(settings().region()).toBe('AT');
      expect(fetch).not.toHaveBeenCalled();
    });
  });

  it('returns to the defaults when disconnected', () => {
    localStorage.setItem(PREFS_KEY, JSON.stringify({ developer: true }));
    disconnect = connectHub({ root: null, session: false });
    expect(ui.developer).toBe(true);
    disconnect();
    disconnect = undefined;
    expect(ui.developer).toBe(false);
  });

  it('resolves `system` to the browser language', () => {
    expect(hubLang({ language: 'system' }, 'de')).toBe('de');
    expect(hubLang({ language: 'en' }, 'de')).toBe('en');
  });
});
