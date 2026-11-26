import { describe, expect, it } from 'vitest';
import { serial } from './serial.ts';

/** A task that settles when `done` or `fail` is called; `started` once it runs. */
function task(log: string[], name: string) {
  let done: () => void = () => {};
  let fail: () => void = () => {};
  const run = () => {
    log.push(`start ${name}`);
    return new Promise<string>((resolve, reject) => {
      done = () => resolve(name);
      fail = () => reject(new Error(name));
    });
  };
  return { run, done: () => done(), fail: () => fail() };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('serial', () => {
  it('starts at once without a running task, otherwise after the previous one settled', async () => {
    const queue = serial();
    const log: string[] = [];
    const a = task(log, 'a');
    const b = task(log, 'b');
    const c = task(log, 'c');
    const ra = queue(a.run);
    expect(log).toEqual(['start a']);
    const rb = queue(b.run);
    const rc = queue(c.run);
    await tick();
    expect(log).toEqual(['start a']);
    // A failed task does not stop the next one.
    a.fail();
    await expect(ra).rejects.toThrow('a');
    await tick();
    expect(log).toEqual(['start a', 'start b']);
    b.done();
    await expect(rb).resolves.toBe('b');
    await tick();
    expect(log).toEqual(['start a', 'start b', 'start c']);
    c.done();
    await expect(rc).resolves.toBe('c');

    // Idle again: the next task starts at once.
    const d = task(log, 'd');
    void queue(d.run);
    expect(log.at(-1)).toBe('start d');
    d.done();
  });
});
