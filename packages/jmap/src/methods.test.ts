import { describe, expect, it } from 'vitest';
import { BLOB, CALENDARS, CONTACTS, CORE, QUOTA, type GetArgs, type GetResponse, type Id } from './core.ts';
import { capabilityOf, registerCapabilities } from './methods.ts';
import { bodyOf, fakeFetch, jmapResponse, sessionJson, testClient } from './testing.ts';

/** Capability of a data type an app adds, as Mail does with `urn:ietf:params:jmap:mail`. */
const NOTES = 'urn:example:jmap:notes';

interface Note {
  id: Id;
  text?: string;
}

// The way an app adds its methods; through the barrel, as with `declare module '@roedy/jmap'`.
declare module './index.ts' {
  interface Methods {
    'Note/get': { args: GetArgs<Note>; response: GetResponse<Note> };
  }
}

describe('capabilityOf', () => {
  it('names the RFC 9404 capability for Blob methods except Blob/copy', () => {
    expect(capabilityOf('Blob/copy')).toBe(CORE);
    expect(capabilityOf('Blob/get')).toBe(BLOB);
    expect(capabilityOf('Foo/get')).toBeUndefined();
  });

  it('names core for Core and PushSubscription, and the quota capability (RFC 9425)', () => {
    expect(capabilityOf('Core/echo')).toBe(CORE);
    expect(capabilityOf('PushSubscription/set')).toBe(CORE);
    expect(capabilityOf('Quota/get')).toBe(QUOTA);
  });

  it('names the RFC 9610 capability for address books and contact cards', () => {
    for (const method of ['AddressBook/get', 'AddressBook/set', 'ContactCard/query', 'ContactCard/changes']) {
      expect(capabilityOf(method)).toBe(CONTACTS);
    }
  });

  it('names the calendars capability for the calendar data types', () => {
    for (const type of ['Calendar', 'CalendarEvent', 'CalendarEventNotification', 'ParticipantIdentity']) {
      expect(capabilityOf(`${type}/get`)).toBe(CALENDARS);
    }
  });

  it('knows no capability for names without a data type', () => {
    expect(capabilityOf('Core')).toBeUndefined();
    expect(capabilityOf('Blobs')).toBeUndefined();
    expect(capabilityOf('')).toBeUndefined();
  });
});

describe('registerCapabilities', () => {
  it('adds the capability of a registered type to using', async () => {
    registerCapabilities({ Note: NOTES });
    expect(capabilityOf('Note/get')).toBe(NOTES);

    const json = sessionJson();
    const session = { ...json, capabilities: { ...(json.capabilities as object), [NOTES]: {} } };
    const server = fakeFetch(
      jmapResponse('s1', ['Note/get', { accountId: 'a1', state: 'n1', list: [{ id: 'n1' }], notFound: [] }, 'c0']),
    );
    const result = await testClient(server.fetch, { session }).call('Note/get', { accountId: 'a1' });
    expect(result.list[0]?.id).toBe('n1');
    expect(bodyOf(server.seen[0])).toMatchObject({ using: [CORE, NOTES] });
  });

  it('accepts the same registration again', () => {
    registerCapabilities({ Note: NOTES, ContactCard: CONTACTS });
    expect(capabilityOf('Note/get')).toBe(NOTES);
    expect(capabilityOf('ContactCard/get')).toBe(CONTACTS);
  });

  it('rejects another capability for a known type and registers nothing then', () => {
    expect(() => registerCapabilities({ Later: NOTES, ContactCard: NOTES })).toThrow(TypeError);
    expect(() => registerCapabilities({ Blob: NOTES })).toThrow(TypeError);
    expect(capabilityOf('ContactCard/get')).toBe(CONTACTS);
    expect(capabilityOf('Blob/get')).toBe(BLOB);
    expect(capabilityOf('Later/get')).toBeUndefined();
  });

  it('rejects invalid type names and empty capabilities', () => {
    for (const type of ['', 'Note/get', 'Not e', '1Note']) {
      expect(() => registerCapabilities({ [type]: NOTES })).toThrow(TypeError);
    }
    expect(() => registerCapabilities({ Empty: '' })).toThrow(TypeError);
    expect(capabilityOf('Empty/get')).toBeUndefined();
  });

  it('leaves unregistered types to Batch.use', async () => {
    const server = fakeFetch(jmapResponse('s1', ['Other/get', { list: [] }, 'c0']));
    const json = sessionJson();
    const session = { ...json, capabilities: { ...(json.capabilities as object), [NOTES]: {} } };
    const batch = testClient(server.fetch, { session }).batch().use(NOTES);
    batch.invoke('Other/get', { accountId: 'a1' });
    await batch.send();
    expect(bodyOf(server.seen[0])).toMatchObject({ using: [CORE, NOTES] });
  });
});
