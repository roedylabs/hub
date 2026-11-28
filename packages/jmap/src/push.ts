/**
 * Push from the JMAP server (RFC 8620, 7): subscriptions with `PushSubscription/get|set`,
 * verification via `verificationCode` (7.2.2), the server's key (RFC 9749)
 * and the messages the server sends. `PushSubscription` has no `accountId`
 * and no state: a subscription belongs to the person the token identifies.
 *
 * Extensions of `PushSubscription` add their properties to
 * `PushSubscriptionExtensions` and their capabilities to the `using` of
 * `createPushSubscription`, and parse their own push messages.
 *
 * Taken over from Roedy Mail (`packages/jmap/src/push.ts`) without `emailPush`
 * (draft-ietf-jmap-emailpush: the `emailPush` property, its capability and
 * the `EmailPush` message), which stays in Mail and plugs in as such an
 * extension.
 */

import type { JmapClient } from './client.ts';
import { WEBPUSH_VAPID, type Id, type PatchObject, type Session, type SetError } from './core.ts';
import { JmapError } from './errors.ts';

export interface PushKeys {
  p256dh: string;
  auth: string;
}

/**
 * Properties that extensions add to `PushSubscription`, via declaration
 * merging, e.g. Mail's `emailPush` (draft-ietf-jmap-emailpush):
 *
 * ```ts
 * declare module '@roedy/jmap' {
 *   interface PushSubscriptionExtensions {
 *     emailPush: Record<Id, EmailPushConfig> | null;
 *   }
 * }
 * ```
 */
// eslint-disable-next-line @typescript-eslint/no-empty-object-type -- filled by declaration merging
export interface PushSubscriptionExtensions {}

export interface JmapPushSubscription extends Partial<PushSubscriptionExtensions> {
  id: Id;
  deviceClientId: string;
  /** Not readable: the server only accepts the URL on create. */
  url: string;
  keys: PushKeys | null;
  /** `null` until verified; Stalwart up to v1.0.x always returns `null`, even after verification. */
  verificationCode: string | null;
  expires: string | null;
  types: string[] | null;
}

export type PushSubscriptionProperty = keyof JmapPushSubscription;

export interface PushSubscriptionGetArgs {
  ids?: Id[] | null;
  properties?: PushSubscriptionProperty[] | null;
}

export interface PushSubscriptionGetResponse {
  list: JmapPushSubscription[];
  notFound: Id[] | null;
}

export interface PushSubscriptionSetArgs {
  create?: Record<Id, Partial<JmapPushSubscription>> | null;
  update?: Record<Id, PatchObject> | null;
  destroy?: Id[] | null;
}

export interface PushSubscriptionSetResponse {
  created: Record<Id, Partial<JmapPushSubscription> & { id: Id }> | null;
  updated: Record<Id, Partial<JmapPushSubscription> | null> | null;
  destroyed: Id[] | null;
  notCreated: Record<Id, SetError> | null;
  notUpdated: Record<Id, SetError> | null;
  notDestroyed: Record<Id, SetError> | null;
}

/** What the client provides on create. */
export interface NewPushSubscription extends Partial<PushSubscriptionExtensions> {
  deviceClientId: string;
  url: string;
  keys: PushKeys;
  /** All data types when omitted. */
  types?: string[];
  /** The server may shorten it, Stalwart to at most 7 days. */
  expires?: string;
}

function refused(error: SetError | undefined): JmapError {
  const type = error?.type ?? 'unknown';
  return new JmapError('request', error?.description ? { type, detail: error.description } : { type });
}

/** The server's `applicationServerKey` (Base64url), `null` without VAPID. */
export function webPushKey(session: Session): string | null {
  const capability = session.capabilities[WEBPUSH_VAPID];
  if (!capability || typeof capability !== 'object') return null;
  const { applicationServerKey } = capability as { applicationServerKey?: unknown };
  return typeof applicationServerKey === 'string' && applicationServerKey ? applicationServerKey : null;
}

/** All subscriptions of the person; `url` and `keys` are missing. */
export async function listPushSubscriptions(client: JmapClient, signal?: AbortSignal): Promise<JmapPushSubscription[]> {
  return (await client.call('PushSubscription/get', { ids: null }, signal)).list;
}

/**
 * Creates a subscription. The server then sends a `PushVerification` to `url`;
 * no messages go out until `verifyPushSubscription` confirms it. `using`
 * names the capabilities of extension properties, e.g. Mail's
 * `urn:ietf:params:jmap:emailpush` with `emailPush`.
 */
export async function createPushSubscription(
  client: JmapClient,
  subscription: NewPushSubscription,
  signal?: AbortSignal,
  using: readonly string[] = [],
): Promise<{ id: Id; expires: string | null }> {
  const batch = client.batch().use(...using);
  const call = batch.call('PushSubscription/set', { create: { push: subscription } });
  const response = (await batch.send(signal)).get(call);
  const created = response.created?.push;
  if (!created) throw refused(response.notCreated?.push);
  return { id: created.id, expires: created.expires ?? null };
}

/** Verifies a subscription with the code from the `PushVerification`. */
export async function verifyPushSubscription(
  client: JmapClient,
  id: Id,
  verificationCode: string,
  signal?: AbortSignal,
): Promise<void> {
  const response = await client.call('PushSubscription/set', { update: { [id]: { verificationCode } } }, signal);
  if (response.notUpdated?.[id]) throw refused(response.notUpdated[id]);
}

/** Renews a subscription. If the server shortens the value, it does not say so; `list` shows the real one. */
export async function renewPushSubscription(
  client: JmapClient,
  id: Id,
  expires: string,
  signal?: AbortSignal,
): Promise<void> {
  const response = await client.call('PushSubscription/set', { update: { [id]: { expires } } }, signal);
  if (response.notUpdated?.[id]) throw refused(response.notUpdated[id]);
}

/** Removes subscriptions; already missing ones count as removed. */
export async function destroyPushSubscriptions(client: JmapClient, ids: Id[], signal?: AbortSignal): Promise<void> {
  if (ids.length === 0) return;
  const response = await client.call('PushSubscription/set', { destroy: ids }, signal);
  const failed = Object.values(response.notDestroyed ?? {}).find((error) => error.type !== 'notFound');
  if (failed) throw refused(failed);
}

/** Message from the server as the service worker receives it decrypted (RFC 8620, 7.1 and 7.2.2). */
export type PushObject =
  | { type: 'PushVerification'; pushSubscriptionId: Id; verificationCode: string }
  | { type: 'StateChange'; changed: Record<Id, Record<string, string>> };

/**
 * Parses a message of RFC 8620; unknown or incomplete ones yield `null`.
 * Messages of extensions (e.g. Mail's `EmailPush`) are for their own parser.
 */
export function parsePushObject(raw: string | null | undefined): PushObject | null {
  let data: unknown;
  try {
    data = raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
  if (!data || typeof data !== 'object') return null;
  const object = data as Record<string, unknown>;
  switch (object['@type']) {
    case 'PushVerification': {
      const { pushSubscriptionId, verificationCode } = object;
      if (typeof pushSubscriptionId !== 'string' || typeof verificationCode !== 'string') return null;
      return { type: 'PushVerification', pushSubscriptionId, verificationCode };
    }
    case 'StateChange': {
      const { changed } = object;
      if (!changed || typeof changed !== 'object') return null;
      return { type: 'StateChange', changed: changed as Record<Id, Record<string, string>> };
    }
    default:
      return null;
  }
}
