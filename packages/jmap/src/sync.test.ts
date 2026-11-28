// Taken over from Roedy Mail (`packages/jmap/src/sync.test.ts`) with address books and contact cards instead of the
// mail data types.
import { describe, expect, it } from 'vitest';
import { applyQueryChanges, changes, queryChanges } from './sync.ts';
import { bodyOf, fakeFetch, jmapResponse, testClient } from './testing.ts';

const page = (
  oldState: string,
  newState: string,
  more: boolean,
  ids: { c?: string[]; u?: string[]; d?: string[] },
) => ({
  accountId: 'a1',
  oldState,
  newState,
  hasMoreChanges: more,
  created: ids.c ?? [],
  updated: ids.u ?? [],
  destroyed: ids.d ?? [],
});

describe('changes', () => {
  it('combines several pages', async () => {
    const server = fakeFetch(
      jmapResponse('s1', ['ContactCard/changes', page('e1', 'e2', true, { c: ['n1', 'n2'], u: ['u1', 'u2'] }), 'c0']),
      jmapResponse('s1', [
        'ContactCard/changes',
        page('e2', 'e3', false, { u: ['n1', 'u3'], d: ['n2', 'u2', 'x1'] }),
        'c1',
      ]),
    );
    const result = await changes(testClient(server.fetch), 'ContactCard', {
      accountId: 'a1',
      sinceState: 'e1',
      maxChanges: 50,
    });

    expect(result).toEqual({
      reset: false,
      oldState: 'e1',
      newState: 'e3',
      created: ['n1'],
      updated: ['u1', 'u3'],
      destroyed: ['u2', 'x1'],
      updatedProperties: null,
      objects: new Map(),
    });
    expect(server.seen.map((s) => bodyOf(s))).toMatchObject([
      { methodCalls: [['ContactCard/changes', { accountId: 'a1', sinceState: 'e1', maxChanges: 50 }, 'c0']] },
      { methodCalls: [['ContactCard/changes', { accountId: 'a1', sinceState: 'e2', maxChanges: 50 }, 'c1']] },
    ]);
  });

  it('fetches the objects in the same request via back-references', async () => {
    const server = fakeFetch(
      jmapResponse(
        's1',
        [
          'AddressBook/changes',
          { ...page('b1', 'b2', false, { c: ['m3'], u: ['m1'] }), updatedProperties: ['sortOrder'] },
          'c0',
        ],
        ['AddressBook/get', { accountId: 'a1', state: 'b2', list: [{ id: 'm3', name: 'New' }], notFound: [] }, 'c1'],
        ['AddressBook/get', { accountId: 'a1', state: 'b2', list: [{ id: 'm1', sortOrder: 4 }], notFound: [] }, 'c2'],
      ),
    );
    const result = await changes(testClient(server.fetch), 'AddressBook', {
      accountId: 'a1',
      sinceState: 'b1',
      fetch: true,
      properties: ['id', 'name', 'sortOrder'],
    });

    expect(result.reset).toBe(false);
    if (result.reset) return;
    expect(result.updatedProperties).toEqual(['sortOrder']);
    expect(result.objects.get('m3')?.name).toBe('New');
    expect(result.objects.get('m1')?.sortOrder).toBe(4);
    const props = ['id', 'name', 'sortOrder'];
    expect(bodyOf(server.seen[0])).toMatchObject({
      methodCalls: [
        ['AddressBook/changes', { accountId: 'a1', sinceState: 'b1', maxChanges: 100 }, 'c0'],
        [
          'AddressBook/get',
          { '#ids': { resultOf: 'c0', name: 'AddressBook/changes', path: '/created' }, properties: props },
          'c1',
        ],
        [
          'AddressBook/get',
          { '#ids': { resultOf: 'c0', name: 'AddressBook/changes', path: '/updated' }, properties: props },
          'c2',
        ],
      ],
    });
  });

  it('asks for at most maxObjectsInGet changes per page when fetching the objects', async () => {
    const get = { accountId: 'a1', state: 'k2', list: [], notFound: [] };
    const reply = (n: number) =>
      jmapResponse(
        's1',
        ['ContactCard/changes', page('k1', 'k2', false, {}), `c${n}`],
        ['ContactCard/get', get, `c${n + 1}`],
        ['ContactCard/get', get, `c${n + 2}`],
      );
    const server = fakeFetch(reply(0), reply(3));
    const client = testClient(server.fetch);
    for (const maxChanges of [500, 20]) {
      await changes(client, 'ContactCard', { accountId: 'a1', sinceState: 'k1', fetch: true, maxChanges });
    }
    expect(server.seen.map((s) => bodyOf(s))).toMatchObject([
      { methodCalls: [['ContactCard/changes', { maxChanges: 100 }, 'c0'], expect.anything(), expect.anything()] },
      { methodCalls: [['ContactCard/changes', { maxChanges: 20 }, 'c3'], expect.anything(), expect.anything()] },
    ]);
  });

  it('reports reset when the server cannot calculate the changes', async () => {
    const server = fakeFetch(jmapResponse('s1', ['error', { type: 'cannotCalculateChanges' }, 'c0']));
    expect(await changes(testClient(server.fetch), 'ContactCard', { accountId: 'a1', sinceState: 'old' })).toEqual({
      reset: true,
    });
  });

  it('aborts when a page makes no progress', async () => {
    const server = fakeFetch(jmapResponse('s1', ['ContactCard/changes', page('e1', 'e1', true, {}), 'c0']));
    await expect(
      changes(testClient(server.fetch), 'ContactCard', { accountId: 'a1', sinceState: 'e1' }),
    ).rejects.toMatchObject({
      kind: 'invalidResponse',
    });
    expect(server.seen).toHaveLength(1);
  });

  it('passes other method errors on', async () => {
    const server = fakeFetch(jmapResponse('s1', ['error', { type: 'accountNotFound' }, 'c0']));
    await expect(
      changes(testClient(server.fetch), 'ContactCard', { accountId: 'x', sinceState: 's' }),
    ).rejects.toMatchObject({
      type: 'accountNotFound',
    });
  });
});

describe('queryChanges', () => {
  it('returns the query changes or reset', async () => {
    const response = {
      accountId: 'a1',
      oldQueryState: 'q1',
      newQueryState: 'q2',
      removed: ['e1'],
      added: [{ id: 'e9', index: 0 }],
    };
    const server = fakeFetch(
      jmapResponse('s1', ['ContactCard/queryChanges', response, 'c0']),
      jmapResponse('s1', ['error', { type: 'tooManyChanges' }, 'c1']),
      jmapResponse('s1', ['error', { type: 'cannotCalculateChanges' }, 'c2']),
    );
    const client = testClient(server.fetch);
    const args = {
      accountId: 'a1',
      filter: { inAddressBook: 'b1' },
      sort: [{ property: 'name/surname' as const, isAscending: true }],
      sinceQueryState: 'q1',
    };
    expect(await queryChanges(client, 'ContactCard/queryChanges', args)).toEqual({ ...response, reset: false });
    expect(await queryChanges(client, 'ContactCard/queryChanges', args)).toEqual({ reset: true });
    expect(await queryChanges(client, 'ContactCard/queryChanges', args)).toEqual({ reset: true });
    expect(bodyOf(server.seen[0])).toMatchObject({ methodCalls: [['ContactCard/queryChanges', args, 'c0']] });
  });
});

describe('applyQueryChanges', () => {
  it('removes first and then inserts by ascending index (RFC 8620, 5.6)', () => {
    const ids = ['a', 'b', 'c', 'd', 'e'];
    const changed = applyQueryChanges(ids, {
      removed: ['b', 'd', 'x'],
      added: [
        { id: 'f', index: 3 },
        { id: 'd', index: 0 },
      ],
    });
    expect(changed).toEqual(['d', 'a', 'c', 'f', 'e']);
    expect(ids).toEqual(['a', 'b', 'c', 'd', 'e']);
  });

  it('drops insertions past the known start', () => {
    expect(applyQueryChanges(['a', 'b'], { removed: [], added: [{ id: 'z', index: 7 }] })).toEqual(['a', 'b']);
  });
});
