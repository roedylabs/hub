/**
 * Apps of the administration for tests (`GET admin/apps`): a paired app with
 * two roles, a pending one and a paired one that is offline. Not a test file.
 */
import type { AdminApp } from '#lib/apps.ts';

const NO_EVENTS = {
  pending: 0,
  attempts: 0,
  failing_since: null,
  retry_at: null,
  last_error: null,
  given_up: false,
} as const;

/** A paired app ("Mail", `/mail/`, roles "User" (default) and "Administrator"); `over` replaces fields. */
export function pairedApp(over: Partial<AdminApp> = {}): AdminApp {
  const slug = over.slug ?? 'mail';
  return {
    id: `app-${slug}`,
    slug,
    internal_url: `http://${slug}.internal:8080`,
    public_url: `https://hub.example.com/${slug}`,
    state: 'paired',
    manifest_id: `roedy.${slug}`,
    version: '1.2.0',
    name: { en: 'Mail', de: 'Mail' },
    description: { en: 'Email for the team', de: 'E-Mail für das Team' },
    home: '/',
    url: `/${slug}/`,
    icon: `/api/v1/apps/${slug}/icon.svg?v=0123456789abcdef`,
    roles: [
      {
        id: 'user',
        name: { en: 'User', de: 'Benutzer' },
        description: { en: 'Reads and sends email', de: 'Liest und sendet E-Mails' },
        default: true,
      },
      {
        id: 'admin',
        name: { en: 'Administrator', de: 'Administrator' },
        description: { en: 'Also manages mail servers', de: 'Verwaltet auch die Mailserver' },
        admin: true,
      },
    ],
    access_default: 'all',
    position: 0,
    health: 'online',
    health_at: '2026-10-26T11:59:00Z',
    online_at: '2026-10-26T11:59:00Z',
    contacted_at: '2026-10-26T11:58:00Z',
    paired_at: '2026-10-01T10:00:00Z',
    created_at: '2026-10-01T09:50:00Z',
    pairing: null,
    user_count: 3,
    events: { ...NO_EVENTS },
    ...over,
  };
}

/** An app waiting for pairing (`wiki`), its code valid until `expires`. */
export function pendingApp(expires = '2026-10-26T12:10:00Z', over: Partial<AdminApp> = {}): AdminApp {
  return pairedApp({
    slug: 'wiki',
    internal_url: 'http://wiki.internal:3000',
    public_url: 'https://hub.example.com/wiki',
    state: 'pending',
    manifest_id: null,
    version: null,
    name: null,
    description: null,
    home: null,
    url: null,
    icon: null,
    roles: null,
    access_default: 'all',
    position: 1,
    health: null,
    health_at: null,
    online_at: null,
    contacted_at: null,
    paired_at: null,
    pairing: { expires_at: expires },
    user_count: 0,
    ...over,
  });
}

/** A paired app ("Notes", `/notes/`) that failed its last health check, events failing. */
export function offlineApp(over: Partial<AdminApp> = {}): AdminApp {
  return pairedApp({
    slug: 'notes',
    name: { en: 'Notes', de: 'Notizen' },
    description: { en: 'Shared notes', de: 'Gemeinsame Notizen' },
    access_default: 'selected',
    position: 2,
    health: 'offline',
    online_at: '2026-10-26T09:00:00Z',
    user_count: 1,
    events: { ...NO_EVENTS, pending: 4, attempts: 3, failing_since: '2026-10-26T10:00:00Z' },
    ...over,
  });
}

/** A pairing code as `POST admin/apps` returns it. */
export const PAIRING = {
  code: 'rhp_7K2MABCDEFGHJKMNPQRSTVWXYZ0123456',
  display: 'rhp_ 7K2M ABCD EFGH JKMN PQRS TVWX YZ01 2345 6',
  expires_at: '2026-10-26T12:15:00Z',
} as const;
