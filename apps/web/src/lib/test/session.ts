/**
 * Body of `GET session` for tests (docs/app-contract.md › `GET /api/v1/session`).
 * Not a test file: imported by the page tests.
 */
import { DEFAULT_PREFS } from '@roedy/ui';
import type { Me, SessionInfo } from '#lib/state/session.svelte.ts';

export function sessionOf(
  user: Partial<Me> & Pick<Me, 'id' | 'email' | 'name'>,
  authTime = 1_795_600_000,
): SessionInfo {
  const full: Me = {
    username: user.email.split('@')[0] ?? 'user',
    role: 'user',
    locale: 'en',
    ...user,
  };
  return {
    user: full,
    prefs: { ...DEFAULT_PREFS },
    session: { id: '01SESSION', auth_time: authTime },
    hub: { name: 'Roedy Hub', home: '/', account: '/account', admin: full.role === 'admin' ? '/admin' : null },
  };
}
