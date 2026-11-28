# @roedy/jmap

JMAP for the browser, shared by the Roedy apps (Mail, Calendar, Contacts): the
apps talk to Stalwart directly with an access token from the hub
(`docs/plan.md` § 9.3). The package holds what every app needs; each app adds
its own data types on top.

```sh
npm install @roedy/jmap
```

Published with every hub release under the hub's version (`0.x` until the
first release), as JavaScript (ES modules) with type declarations
(CONTRIBUTING.md › Packages and crates).

## Contents

| Module        | What it does                                                                                                                         |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `core.ts`     | Types of RFC 8620 (session, requests, standard methods, filters) and the capability constants                                        |
| `client.ts`   | `JmapClient`: session, batches with back-references, blobs, EventSource; token per request, retry on 401, limits of the server        |
| `session.ts`  | Session validation, same-origin URLs, server limits, primary accounts, URL templates                                                 |
| `token.ts`    | `TokenSource`: caches the broker's access token, renews it on 401, honours `Retry-After`                                              |
| `methods.ts`  | `Methods` (method → arguments and response) and the capability per data type (`capabilityOf`, `registerCapabilities`)              |
| `sync.ts`     | `changes()` across pages (optionally with the objects), `queryChanges()`, `applyQueryChanges()`                                      |
| `stream.ts`   | Push via EventSource over `fetch` (with Authorization), reconnect with backoff, `ping` watchdog                                      |
| `push.ts`     | Web Push subscriptions (`PushSubscription`, RFC 8620, 7), the server's VAPID key (RFC 9749), parsing of push messages               |
| `errors.ts`   | `JmapError` (transport, rejected requests) and `MethodError` (single method calls)                                                   |
| `contacts.ts` | `AddressBook` and `ContactCard` (RFC 9610, JSContact RFC 9553), as far as used                                                       |
| `testing.ts`  | For tests only (not exported): a fake JMAP server behind `fetch` and a fixed token provider                                          |

The methods of address books and contact cards are typed in `Methods`. For
calendars (draft-ietf-jmap-calendars) there is so far only the capability
`CALENDARS` and its data types in the capability map (`Calendar`,
`CalendarEvent`, `CalendarEventNotification`, `ParticipantIdentity`); the
Calendar app adds their types.

## Extending it in an app

A batch puts the capability of every method's data type into the request's
`using`. An app adds its methods to `Methods` by declaration merging and the
capabilities of its data types with `registerCapabilities`, once, next to the
types (registering the same again is fine, another capability for a known type
throws):

```ts
import { registerCapabilities, type GetArgs, type GetResponse } from '@roedy/jmap';

export const MAIL = 'urn:ietf:params:jmap:mail';

declare module '@roedy/jmap' {
  interface Methods {
    'Mailbox/get': { args: GetArgs<Mailbox>; response: GetResponse<Mailbox> };
  }
}

registerCapabilities({ Mailbox: MAIL, Thread: MAIL, Email: MAIL });
```

`client.call('Mailbox/get', …)` is then typed and sends `using: [core, mail]`.
For a single request, `batch.use(capability)` adds a capability by hand.

Extensions of `PushSubscription` work the same way: their properties go into
`PushSubscriptionExtensions` by declaration merging, their capabilities into
the `using` argument of `createPushSubscription`, and they parse their own push
messages (`parsePushObject` knows only `PushVerification` and `StateChange`).

## What stays in Mail

JMAP for mail (RFC 8621: Mailbox, Thread, Email, SearchSnippet, Identity,
EmailSubmission, VacationResponse, with their capabilities and account
capabilities such as `maxSizeAttachmentsPerEmail` and `maxDelayedSend`),
Sieve (RFC 9661) and push for new mail (`emailPush` and the `EmailPush`
message, draft-ietf-jmap-emailpush). Mail adds them on top of this package as
shown above.

## Origin

Taken over from `packages/jmap` of
[roedylabs/mail](https://github.com/roedylabs/mail) at commit `d5bdf14`
(v0.5.0), without `mail.ts` and `sieve.ts` and the mail parts of the other
modules. Every module notes in its doc comment what changed.
