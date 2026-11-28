/**
 * Methods with their arguments and responses, and the capability each data
 * type needs in a request's `using`.
 *
 * Apps add their own methods to `Methods` via declaration merging and the
 * capabilities of their data types with `registerCapabilities`, e.g. Mail:
 *
 * ```ts
 * declare module '@roedy/jmap' {
 *   interface Methods {
 *     'Mailbox/get': { args: GetArgs<Mailbox>; response: GetResponse<Mailbox> };
 *   }
 * }
 *
 * registerCapabilities({ Mailbox: MAIL });
 * ```
 *
 * Taken over from Roedy Mail (`packages/jmap/src/methods.ts`) without the
 * methods of Mailbox, Thread, Email, SearchSnippet, Identity,
 * EmailSubmission, VacationResponse and SieveScript, which stay in Mail. The
 * fixed table of capabilities became a registry; calendars, address books and
 * the further contact card methods are new.
 */

import {
  BLOB,
  CALENDARS,
  CONTACTS,
  CORE,
  QUOTA,
  type ChangesArgs,
  type ChangesResponse,
  type GetArgs,
  type GetResponse,
  type QueryArgs,
  type QueryChangesArgs,
  type QueryChangesResponse,
  type QueryResponse,
  type SetArgs,
  type SetResponse,
} from './core.ts';
import type { AddressBook, ContactCard, ContactCardFilter, ContactCardSortProperty } from './contacts.ts';
import type {
  PushSubscriptionGetArgs,
  PushSubscriptionGetResponse,
  PushSubscriptionSetArgs,
  PushSubscriptionSetResponse,
} from './push.ts';

interface Method<A, R> {
  args: A;
  response: R;
}

export interface Methods {
  'Core/echo': Method<Record<string, unknown>, Record<string, unknown>>;

  'PushSubscription/get': Method<PushSubscriptionGetArgs, PushSubscriptionGetResponse>;
  'PushSubscription/set': Method<PushSubscriptionSetArgs, PushSubscriptionSetResponse>;

  'AddressBook/get': Method<GetArgs<AddressBook>, GetResponse<AddressBook>>;
  'AddressBook/changes': Method<ChangesArgs, ChangesResponse>;
  'AddressBook/set': Method<SetArgs<AddressBook>, SetResponse<AddressBook>>;

  'ContactCard/get': Method<GetArgs<ContactCard>, GetResponse<ContactCard>>;
  'ContactCard/changes': Method<ChangesArgs, ChangesResponse>;
  'ContactCard/query': Method<QueryArgs<ContactCardFilter, ContactCardSortProperty>, QueryResponse>;
  'ContactCard/queryChanges': Method<
    QueryChangesArgs<ContactCardFilter, ContactCardSortProperty>,
    QueryChangesResponse
  >;
  'ContactCard/set': Method<SetArgs<ContactCard>, SetResponse<ContactCard>>;
}

export type MethodName = Extract<keyof Methods, string>;
export type ArgsOf<M extends MethodName> = Methods[M]['args'];
export type ResponseOf<M extends MethodName> = Methods[M]['response'];

/** Capability per data type (part before the `/`), for the request's `using`. */
const capabilities = new Map<string, string>([
  ['Core', CORE],
  ['Blob', BLOB],
  ['PushSubscription', CORE],
  ['Quota', QUOTA],
  ['AddressBook', CONTACTS],
  ['ContactCard', CONTACTS],
  ['Calendar', CALENDARS],
  ['CalendarEvent', CALENDARS],
  ['CalendarEventNotification', CALENDARS],
  ['ParticipantIdentity', CALENDARS],
]);

/**
 * Adds the capabilities of further data types, e.g. `{ Mailbox: MAIL }`;
 * batches then add them to `using` by themselves. Registering a type again
 * with the same capability changes nothing, with another one throws
 * `TypeError`.
 */
export function registerCapabilities(types: Readonly<Record<string, string>>): void {
  const entries = Object.entries(types);
  for (const [type, capability] of entries) {
    if (!/^[A-Za-z][A-Za-z0-9]*$/.test(type)) throw new TypeError(`invalid data type: ${type}`);
    if (capability === '') throw new TypeError(`${type}: empty capability`);
    const known = capabilities.get(type);
    if (known !== undefined && known !== capability) {
      throw new TypeError(`${type} already needs ${known}, not ${capability}`);
    }
  }
  // Only after all checks, so that a rejected call registers nothing.
  for (const [type, capability] of entries) capabilities.set(type, capability);
}

/** Capability a method needs in `using`, or `undefined` for unknown data types. */
export function capabilityOf(method: string): string | undefined {
  // `Blob/copy` belongs to core, the other Blob methods to RFC 9404.
  if (method === 'Blob/copy') return CORE;
  const slash = method.indexOf('/');
  return slash < 0 ? undefined : capabilities.get(method.slice(0, slash));
}
