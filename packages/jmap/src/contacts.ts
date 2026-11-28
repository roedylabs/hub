/**
 * JMAP for Contacts (RFC 9610): `AddressBook` and `ContactCard` as a
 * JSContact card (RFC 9553), so far only the properties the apps use.
 *
 * Taken over from Roedy Mail (`packages/jmap/src/contacts.ts`); `AddressBook`
 * and `ContactCard.addressBookIds` are new.
 */

import type { Id } from './core.ts';

/** Address book (RFC 9610, 2); `AddressBook/get` returns only the requested properties. */
export interface AddressBook {
  id: Id;
  name?: string;
  description?: string | null;
  sortOrder?: number;
  isDefault?: boolean;
  isSubscribed?: boolean;
}

/** Part of a name (RFC 9553, 2.2.1.2), e.g. `given` or `surname`. */
export interface NameComponent {
  kind: string;
  value: string;
}

/** Name of a card (RFC 9553, 2.2.1). */
export interface ContactName {
  full?: string;
  components?: NameComponent[];
}

/** Email address of a card (RFC 9553, 2.3.1). */
export interface ContactEmail {
  address: string;
  /** Preference, 1 = highest (RFC 9553, 1.5.4). */
  pref?: number;
}

/** Contact card (RFC 9610, 3); `ContactCard/get` returns only the requested properties. */
export interface ContactCard {
  id: Id;
  /** Address books the card belongs to (RFC 9610, 3.1). */
  addressBookIds?: Record<Id, true>;
  name?: ContactName | null;
  emails?: Record<string, ContactEmail> | null;
}

/** Filter of `ContactCard/query` (RFC 9610, 3.5.1), as far as used. */
export interface ContactCardFilter {
  inAddressBook?: Id;
  text?: string;
  name?: string;
  email?: string;
}

export type ContactCardSortProperty = 'created' | 'updated' | 'name/given' | 'name/surname';
