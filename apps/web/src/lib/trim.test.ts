import { describe, expect, it } from 'vitest';
import { trimWs } from './trim.ts';

describe('trimWs', () => {
  it('trims Unicode White_Space at both ends like Rust str::trim', () => {
    expect(trimWs('  New Ltd \t\n')).toBe('New Ltd');
    expect(trimWs('\u0085New Ltd\u0085')).toBe('New Ltd');
    expect(trimWs('  　x  ')).toBe('x');
    expect(trimWs(' \u0085 ')).toBe('');
  });

  it('keeps whitespace inside and the byte order mark, which Rust does not trim', () => {
    expect(trimWs(' New\u0085Ltd ')).toBe('New\u0085Ltd');
    expect(trimWs('﻿x﻿')).toBe('﻿x﻿');
  });
});
