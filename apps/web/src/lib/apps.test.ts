import { afterEach, describe, expect, it } from 'vitest';
import {
  accessOptions,
  appName,
  appStatus,
  defaultAccess,
  defaultRole,
  eventsProblem,
  roleName,
  textIn,
} from '#lib/apps.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { disabledApp, offlineApp, pairedApp, pendingApp } from '#lib/test/apps.ts';

const NOW = Date.parse('2026-10-26T12:00:00Z');

afterEach(() => prefs.reset());

describe('apps', () => {
  it('takes texts in the interface’s language, otherwise English', () => {
    expect(textIn({ en: 'Mail', de: 'Post' })).toBe('Mail');
    prefs.set({ lang: 'de' });
    expect(textIn({ en: 'Mail', de: 'Post' })).toBe('Post');
    expect(textIn({ en: 'Only' })).toBe('Only');
    expect(textIn(null, 'fallback')).toBe('fallback');
    // Remembered on the device: back to English for the other tests.
    prefs.set({ lang: 'en' });
  });

  it('names a pending app by its slug', () => {
    expect(appName(pairedApp())).toBe('Mail');
    expect(appName(pendingApp())).toBe('wiki');
  });

  it('knows the default role, role names and what a new user gets', () => {
    expect(defaultRole(pairedApp())?.id).toBe('user');
    expect(defaultRole(pendingApp())).toBeNull();
    expect(roleName(pairedApp(), 'admin')).toBe('Administrator');
    expect(roleName(pairedApp(), 'gone')).toBe('gone');
    expect(defaultAccess(pairedApp())).toBe('user');
    expect(defaultAccess(offlineApp())).toBeNull();
  });

  it('offers "No access" and the roles, the current one checked', () => {
    expect(accessOptions(pairedApp(), 'admin').map((o) => [o.value, o.label, o.isDefault, o.on])).toEqual([
      [null, 'No access', false, false],
      ['user', 'User', true, false],
      ['admin', 'Administrator', false, true],
    ]);
    expect(accessOptions(pairedApp(), null)[0]?.on).toBe(true);
    expect(accessOptions(pairedApp(), null)[1]?.desc).toBe('Reads and sends email');
  });

  it('describes the status: online, offline, not checked, pending with its code', () => {
    expect(appStatus(pairedApp(), NOW)).toMatchObject({
      kind: 'online',
      label: 'Online',
      detail: 'Last contact 2 min ago',
    });
    expect(appStatus(offlineApp(), NOW)).toMatchObject({
      kind: 'offline',
      label: 'Offline',
      detail: 'Last online 3 hours ago',
    });
    expect(appStatus(offlineApp({ online_at: null }), NOW).detail).toBe('Never online yet');
    expect(appStatus(pairedApp({ health: null, contacted_at: null }), NOW)).toMatchObject({
      kind: 'unknown',
      detail: '',
    });
    expect(appStatus(pendingApp('2026-10-26T12:09:30Z'), NOW)).toMatchObject({
      kind: 'pending',
      label: 'Waiting for pairing',
      detail: 'Pairing code valid for 10 min',
    });
    expect(appStatus(pendingApp('2026-10-26T11:00:00Z'), NOW).detail).toBe('Pairing code expired');
    expect(appStatus(pendingApp('', { pairing: null }), NOW).detail).toBe('Pairing code expired');
  });

  it('puts "Switched off" before every other status', () => {
    expect(appStatus(disabledApp(), NOW)).toMatchObject({
      kind: 'disabled',
      label: 'Switched off',
      detail: 'Switched off 1 hour ago',
      icon: 'ti-plug-off',
    });
    expect(appStatus(disabledApp({ health: 'offline' }), NOW).kind).toBe('disabled');
    expect(appStatus(pendingApp('2026-10-26T12:09:30Z', { enabled: false }), NOW).kind).toBe('disabled');
    expect(appStatus(disabledApp({ disabled_at: null }), NOW).detail).toBe('');
    // Its events wait on purpose: no problem to report.
    expect(eventsProblem(disabledApp(), NOW)).toBe('');
  });

  it('reports failing event deliveries', () => {
    expect(eventsProblem(pairedApp(), NOW)).toBe('');
    expect(eventsProblem(offlineApp(), NOW)).toBe('Events can’t be delivered (first failure 2 hours ago, 4 waiting).');
    const given = offlineApp({ events: { ...offlineApp().events, given_up: true, retry_at: null } });
    expect(eventsProblem(given, NOW)).toBe(
      'Events haven’t been delivered for 7 days. The hub waits until the app is back.',
    );
  });
});
