// Taken over from Roedy Mail (`apps/web/src/lib/developer.test.ts`), only `debugBarValues`, with the distance as a
// parameter and the view already without secrets.
import { describe, expect, it } from 'vitest';
import { DEBUG_BOTTOM, debugBarValues } from './developer.ts';
import type { ErrorEnv } from './errors.ts';

const ENV: ErrorEnv = {
  version: '0.7.0',
  build: 'abc1234',
  mobile: false,
  width: 1440.4,
  height: 899.6,
  path: '/admin/developer',
  browser: 'Test Browser',
  timeZone: 'Europe/Berlin',
  language: 'de',
  region: 'Germany',
};

describe('debugBarValues', () => {
  it('shows layout, rounded window size and view, at the bottom', () => {
    expect(DEBUG_BOTTOM).toBe('18px');
    expect(debugBarValues(ENV)).toEqual({ text: 'Desktop · 1440×900 · /admin/developer', bottom: '18px' });
  });

  it('names the narrow layout and sits where the app says, e.g. above a tab bar', () => {
    expect(debugBarValues({ ...ENV, mobile: true }, '90px')).toEqual({
      text: 'Narrow · 1440×900 · /admin/developer',
      bottom: '90px',
    });
  });
});
