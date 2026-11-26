// Taken over from Roedy Mail (`apps/web/src/lib/ago.test.ts`), with the cases of the hub's former page helpers.
import { describe, expect, it } from 'vitest';
import { ago } from './ago.ts';

const NOW = Date.parse('2026-10-26T12:00:00Z');

describe('ago', () => {
  it('names time spans relative to now', () => {
    expect(ago(NOW - 30_000, NOW)).toBe('just now');
    expect(ago(NOW - 60_000, NOW)).toBe('1 min ago');
    expect(ago(NOW - 5 * 60_000, NOW)).toBe('5 min ago');
    expect(ago(NOW - 3600_000, NOW)).toBe('1 hour ago');
    expect(ago(NOW - 47 * 3600_000, NOW)).toBe('47 hours ago');
    expect(ago(NOW - 49 * 3600_000, NOW)).toBe('2 days ago');
    expect(ago(NOW - 72 * 3600_000, NOW)).toBe('3 days ago');
    expect(ago(NOW - 10 * 24 * 3600_000, NOW)).toBe('10 days ago');
  });

  it('uses now by default', () => {
    expect(ago(Date.now())).toBe('just now');
  });
});
