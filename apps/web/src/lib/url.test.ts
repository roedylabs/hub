// Taken over from Roedy Mail (`apps/web/src/lib/url.test.ts`) without `normUrl`.
import { describe, expect, it } from 'vitest';
import { domainsOf, hostOf, httpOther, isDomain, localHttp, originOf } from './url.ts';

describe('Addresses', () => {
  it('returns host with port and origin only for http(s) addresses', () => {
    expect(hostOf('https://mail.example.com:8443/path')).toBe('mail.example.com:8443');
    expect(hostOf('https://mail.example.com:443')).toBe('mail.example.com');
    expect(hostOf('ftp://mail.example.com')).toBe('');
    expect(hostOf('https://')).toBe('');
    expect(originOf('http://localhost:5173/x')).toBe('http://localhost:5173');
    expect(originOf('not a url')).toBe('');
  });

  it('allows http:// only on localhost and exactly 127.0.0.1', () => {
    expect(localHttp('http://localhost:8080')).toBe(true);
    expect(localHttp('http://127.0.0.1')).toBe(true);
    expect(localHttp('http://LOCALHOST')).toBe(true);
    expect(localHttp('https://localhost')).toBe(false);
    expect(localHttp('http://127.0.0.2')).toBe(false);
    expect(httpOther('http://stalwart:8080')).toBe(true);
    expect(httpOther('http://localhost:8080')).toBe(false);
    expect(httpOther('https://stalwart')).toBe(false);
    expect(httpOther('nonsense')).toBe(false);
  });
});

describe('Domains', () => {
  it('accepts domains with the label rules of the backend', () => {
    for (const d of ['example.com', 'mail-1.example.com', 'a.b', 'x'.repeat(63) + '.test'])
      expect(isDomain(d), d).toBe(true);
    for (const d of [
      'example',
      '-bad.com',
      'bad-.com',
      'a.-b.com',
      'a..com',
      'bad_domain.com',
      'x'.repeat(64) + '.test',
    ])
      expect(isDomain(d), d).toBe(false);
    expect(isDomain(`${'a'.repeat(63)}.${'b'.repeat(63)}.${'c'.repeat(63)}.${'d'.repeat(61)}`)).toBe(true);
    expect(isDomain(`${'a'.repeat(63)}.${'b'.repeat(63)}.${'c'.repeat(63)}.${'d'.repeat(62)}`)).toBe(false);
  });

  it('splits domains at commas, semicolons and whitespace, lower case and without a leading @', () => {
    expect(domainsOf(' @Example.com, example.org;b.test  c.test ')).toEqual([
      'example.com',
      'example.org',
      'b.test',
      'c.test',
    ]);
    expect(domainsOf(' , ; ')).toEqual([]);
  });
});
