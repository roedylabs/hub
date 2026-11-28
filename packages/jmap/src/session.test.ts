// Taken over from Roedy Mail (`packages/jmap/src/session.test.ts`) without the tests of `mailCapability` and
// `submissionCapability`, with contacts and calendars instead of mail and vacation responses.
import { describe, expect, it } from 'vitest';
import { CALENDARS, CONTACTS, CORE } from './core.ts';
import { retryAfterSeconds } from './errors.ts';
import { coreCapability, expandTemplate, hasAccountCapability, parseSession, primaryAccount } from './session.ts';
import { SERVER, sessionJson } from './testing.ts';

describe('parseSession', () => {
  it('resolves relative URLs and keeps the template placeholders', () => {
    const session = parseSession(
      sessionJson({
        apiUrl: '/jmap/',
        downloadUrl: '/jmap/download/{accountId}/{blobId}/{name}?accept={type}',
        uploadUrl: '//jmap.example.test/jmap/upload/{accountId}/',
        eventSourceUrl: 'eventsource/?types={types}',
      }),
      `${SERVER}/jmap/session`,
    );
    expect(session.apiUrl).toBe(`${SERVER}/jmap/`);
    expect(session.downloadUrl).toBe(`${SERVER}/jmap/download/{accountId}/{blobId}/{name}?accept={type}`);
    expect(session.uploadUrl).toBe(`${SERVER}/jmap/upload/{accountId}/`);
    expect(session.eventSourceUrl).toBe(`${SERVER}/jmap/eventsource/?types={types}`);
  });

  it('rejects URLs on another origin', () => {
    for (const apiUrl of [
      'https://jmap.example.test:8443/jmap/',
      'http://jmap.example.test/jmap/',
      '//other.example.test/',
    ]) {
      expect(() => parseSession(sessionJson({ apiUrl }), SERVER)).toThrow(
        expect.objectContaining({ kind: 'foreignOrigin' }),
      );
    }
    expect(() => parseSession(sessionJson({ downloadUrl: 'https://cdn.example.test/{blobId}' }), SERVER)).toThrow(
      expect.objectContaining({ kind: 'foreignOrigin' }),
    );
  });

  it('requires Core and apiUrl, everything else is optional', () => {
    expect(() => parseSession({ capabilities: {}, apiUrl: '/jmap/' }, SERVER)).toThrow(
      expect.objectContaining({ kind: 'invalidResponse' }),
    );
    expect(() => parseSession({ capabilities: { [CORE]: {} } }, SERVER)).toThrow(
      expect.objectContaining({ kind: 'invalidResponse' }),
    );
    const anonymous = parseSession({ capabilities: { [CORE]: {} }, apiUrl: '/jmap/' }, SERVER);
    expect(anonymous).toMatchObject({ accounts: {}, primaryAccounts: {}, username: '', downloadUrl: '', state: '' });
    expect(() => primaryAccount(anonymous, CORE)).toThrow(expect.objectContaining({ kind: 'unsupported' }));
  });

  it('returns the primary account only with the capability', () => {
    const session = parseSession(sessionJson({ capabilities: { [CORE]: {} } }), SERVER);
    expect(() => primaryAccount(session, CONTACTS)).toThrow(
      expect.objectContaining({ details: { capability: CONTACTS } }),
    );
    expect(primaryAccount(parseSession(sessionJson(), SERVER), CONTACTS)).toBe('a1');
  });

  it('fills missing limits with the minimums from RFC 8620', () => {
    const core = coreCapability(
      parseSession({ capabilities: { [CORE]: { maxCallsInRequest: 32 } }, apiUrl: '/' }, SERVER),
    );
    expect(core).toMatchObject({ maxCallsInRequest: 32, maxConcurrentRequests: 4, maxObjectsInGet: 500 });
  });

  it('knows an account capability only with an entry in session and account', () => {
    const account = (caps: Record<string, unknown>) => ({
      name: 'user@example.test',
      isPersonal: true,
      isReadOnly: false,
      accountCapabilities: caps,
    });
    const session = parseSession(
      sessionJson({
        capabilities: { ...(sessionJson().capabilities as Record<string, unknown>), [CALENDARS]: {} },
        accounts: { a1: account({ [CONTACTS]: {}, [CALENDARS]: {} }), a2: account({ [CONTACTS]: {} }) },
      }),
      SERVER,
    );
    expect(hasAccountCapability(session, 'a1', CALENDARS)).toBe(true);
    expect(hasAccountCapability(session, 'a2', CALENDARS)).toBe(false);
    expect(hasAccountCapability(session, 'missing', CALENDARS)).toBe(false);
    // Without an entry in the session the account does not count.
    const without = parseSession(sessionJson({ accounts: { a1: account({ [CALENDARS]: {} }) } }), SERVER);
    expect(hasAccountCapability(without, 'a1', CALENDARS)).toBe(false);
  });
});

describe('helpers', () => {
  it('encodes template values per RFC 6570', () => {
    expect(
      expandTemplate('/d/{blobId}/{name}?t={type}&x={missing}', {
        blobId: 'a+b',
        name: "it's (1)*",
        type: 'text/plain',
      }),
    ).toBe('/d/a%2Bb/it%27s%20%281%29%2A?t=text%2Fplain&x=');
  });

  it('reads Retry-After as seconds or HTTP date', () => {
    const now = Date.parse('2026-10-27T12:00:00Z');
    expect(retryAfterSeconds('120', now)).toBe(120);
    expect(retryAfterSeconds('Tue, 27 Oct 2026 12:00:30 GMT', now)).toBe(30);
    expect(retryAfterSeconds('Tue, 27 Oct 2026 11:00:00 GMT', now)).toBe(0);
    expect(retryAfterSeconds('soon', now)).toBeUndefined();
    expect(retryAfterSeconds(null, now)).toBeUndefined();
  });
});
