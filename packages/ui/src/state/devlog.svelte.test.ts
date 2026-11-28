// Taken over from Roedy Mail (`apps/web/src/lib/state/devlog.svelte.test.ts`) with the level as a field
// (`visibleLog` from Mail's `developer.test.ts`); the toasts in the log are tested with `ui` (`ui.test.ts`).
import { flushSync } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DevLog, LOG_LEVELS, LOG_MAX, visibleLog, type LogEntry } from './devlog.svelte.ts';

afterEach(() => {
  vi.useRealTimers();
});

describe('DevLog', () => {
  it('puts new entries in front and records the timestamp', () => {
    vi.useFakeTimers({ now: 5000 });
    const log = new DevLog();
    log.add('info', 'One');
    log.add('error', 'Two');
    expect(log.entries.map((e) => e.msg)).toEqual(['Two', 'One']);
    expect(log.entries[0]).toMatchObject({ level: 'error', t: 5000 });
    expect(log.entries[0]?.id).not.toBe(log.entries[1]?.id);
  });

  it('keeps at most 120 entries and drops the oldest', () => {
    const log = new DevLog();
    for (let i = 0; i < LOG_MAX + 5; i++) log.add('debug', `Entry ${i}`);
    expect(log.entries).toHaveLength(LOG_MAX);
    expect(log.entries[0]?.msg).toBe(`Entry ${LOG_MAX + 4}`);
    expect(log.entries.at(-1)?.msg).toBe('Entry 5');
  });

  it('clears the log', () => {
    const log = new DevLog();
    log.add('info', 'One');
    log.clear();
    expect(log.entries).toEqual([]);
  });

  it('does not make an effect that writes depend on the log', () => {
    const log = new DevLog();
    let runs = 0;
    const stop = $effect.root(() => {
      $effect(() => {
        runs++;
        log.add('info', 'From the effect');
      });
    });
    flushSync();
    log.add('info', 'Afterwards');
    flushSync();
    stop();
    expect(runs).toBe(1);
  });

  it('records errors and unhandled promises until detached', () => {
    const log = new DevLog();
    const target = new EventTarget() as unknown as Window;
    const detach = log.attach(target);
    target.dispatchEvent(Object.assign(new Event('error'), { message: 'Broken' }));
    target.dispatchEvent(Object.assign(new Event('error'), { message: '' }));
    target.dispatchEvent(Object.assign(new Event('unhandledrejection'), { reason: new Error('Rejected') }));
    target.dispatchEvent(Object.assign(new Event('unhandledrejection'), { reason: 'As text' }));
    target.dispatchEvent(Object.assign(new Event('unhandledrejection'), { reason: { code: 1 } }));
    expect(log.entries.map((e) => [e.level, e.msg])).toEqual([
      ['error', 'Unknown error'],
      ['error', 'As text'],
      ['error', 'Rejected'],
      ['error', 'Unknown error'],
      ['error', 'Broken'],
    ]);
    detach();
    target.dispatchEvent(Object.assign(new Event('error'), { message: 'Later' }));
    expect(log.entries).toHaveLength(5);
  });
});

describe('log level', () => {
  const entries: LogEntry[] = LOG_LEVELS.map((level, i) => ({ id: i + 1, t: 0, level, msg: level }));

  it('shows a level and everything more severe', () => {
    expect(visibleLog(entries, 'error').map((e) => e.level)).toEqual(['error']);
    expect(visibleLog(entries, 'info').map((e) => e.level)).toEqual(['error', 'warning', 'info']);
    expect(visibleLog(entries, 'debug')).toHaveLength(4);
  });

  it('follows the level the app sets, by default info', () => {
    const log = new DevLog();
    log.add('debug', 'Details');
    log.add('warning', 'Careful');
    expect(log.level).toBe('info');
    expect(log.visible.map((e) => e.msg)).toEqual(['Careful']);
    log.level = 'debug';
    expect(log.visible.map((e) => e.msg)).toEqual(['Careful', 'Details']);
    log.level = 'error';
    expect(log.visible).toEqual([]);
  });
});
