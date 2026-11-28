/**
 * Stalwart servers and accounts of the tests (`GET stalwart/servers`, `GET stalwart/accounts`). Not a test file.
 */
import type { StalwartAccount } from '@roedy/ui';
import type { PublicServer, PublicServers } from '#lib/stalwart.ts';

export const SERVER: PublicServer = {
  id: '01JSRV1',
  label: 'Mail',
  url: 'https://mail.example.com',
  host: 'mail.example.com',
  domains: ['example.com'],
  sign_in: true,
  reach: 'ok',
  cors: 'ok',
  oauth: 'ok',
};

/** A registry server without the hub's OAuth client: access tokens only. */
export const TOKEN_SERVER: PublicServer = {
  id: '01JSRV2',
  label: 'Archive',
  url: 'https://archive.example.org',
  host: 'archive.example.org',
  domains: ['example.org'],
  sign_in: false,
  reach: 'ok',
  cors: 'ok',
  oauth: 'unk',
};

export function serversOf(servers: PublicServer[] = [SERVER, TOKEN_SERVER], allow = false): PublicServers {
  return { servers, allow_custom_servers: allow };
}

export function accountOf(patch: Partial<StalwartAccount> = {}): StalwartAccount {
  return {
    id: '01JACCT1',
    email: 'alex@example.com',
    username: 'alex@example.com',
    url: 'https://mail.example.com',
    server: { id: SERVER.id, label: SERVER.label },
    auth: 'oauth',
    session_url: 'https://mail.example.com/jmap/session',
    capabilities: ['urn:ietf:params:jmap:core', 'urn:ietf:params:jmap:mail', 'urn:ietf:params:jmap:contacts'],
    default: true,
    color: 0,
    state: 'ok',
    quota: { used: 512 * 1024 ** 2, total: 2 * 1024 ** 3, checked_at: '2026-11-28T09:00:00.000Z' },
    created_at: '2026-11-20T08:00:00.000Z',
    ...patch,
  };
}
