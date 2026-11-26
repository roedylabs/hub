/**
 * axe-core over Administration › Email in every state (loading, load failed,
 * connected, not set up, no connection, token sign-in with unsaved changes,
 * field errors, saving), desktop and mobile, light and dark. Colour contrast
 * is left to the browser audit (scripts/a11y/audit.mjs).
 */
import { fireEvent, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { NO_MAIL, type SystemMail } from '#lib/system-mail.svelte.ts';
import { violations } from '#lib/test/axe.ts';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

afterEach(() => {
  resetSignedIn();
  delete document.documentElement.dataset.mode;
});

const STORED: SystemMail = {
  url: 'https://mail.example.com',
  auth: 'password',
  user: 'noreply@example.com',
  has_secret: true,
  identity_id: 'i1',
  sender_name: null,
  reply_to: null,
  keep_sent: true,
  status: 'ok',
  reason: null,
  checked_at: new Date(Date.now() - 5 * 60_000).toISOString(),
  identities: [{ id: 'i1', name: 'Example Org', email: 'noreply@example.com' }],
};

const field = (label: string) => screen.getByLabelText(label, { selector: 'input' });
const ready = () =>
  vi.waitFor(() => expect(screen.getByRole('heading', { level: 2, name: 'Status' })).toBeInTheDocument());
const never = () => new Promise<never>(() => {});

/** States; `api` builds fresh queues per test (`mockApi` consumes them). */
const STATES: Record<string, { api: () => ApiRoutes; prepare?: () => Promise<unknown> }> = {
  loading: { api: () => ({ 'GET admin/system-mail': [never] }) },
  failed: {
    api: () => ({ 'GET admin/system-mail': [{ status: 500, body: { error: 'internal' } }] }),
    prepare: () => vi.waitFor(() => expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument()),
  },
  connected: { api: () => ({ 'GET admin/system-mail': [{ body: STORED }] }), prepare: ready },
  'not set up': { api: () => ({ 'GET admin/system-mail': [{ body: NO_MAIL }] }), prepare: ready },
  'no connection': {
    api: () => ({ 'GET admin/system-mail': [{ body: { ...STORED, status: 'err', reason: 'auth_failed' } }] }),
    prepare: ready,
  },
  'token, unsaved': {
    api: () => ({ 'GET admin/system-mail': [{ body: STORED }] }),
    prepare: async () => {
      await ready();
      await fireEvent.click(screen.getByRole('button', { name: 'Token' }));
    },
  },
  'field errors': {
    api: () => ({ 'GET admin/system-mail': [{ body: STORED }] }),
    prepare: async () => {
      await ready();
      await fireEvent.input(field('Server address'), { target: { value: 'mail.example.com' } });
      await fireEvent.input(field('Reply-to address'), { target: { value: 'no-at' } });
      await fireEvent.input(field('Sender name'), { target: { value: 'x'.repeat(101) } });
    },
  },
  saving: {
    api: () => ({ 'GET admin/system-mail': [{ body: STORED }], 'PUT admin/system-mail': [never] }),
    prepare: async () => {
      await ready();
      await fireEvent.input(field('Password'), { target: { value: 'new-pw' } });
      await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
      await vi.waitFor(() => expect(document.querySelector('button[aria-busy="true"]')).not.toBeNull());
    },
  },
};

describe('Email page – axe', () => {
  for (const mobile of [false, true])
    for (const mode of ['light', 'dark'] as const)
      for (const [state, { api, prepare }] of Object.entries(STATES))
        it(`${state}, ${mobile ? 'mobile' : 'desktop'}, ${mode}`, async () => {
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode });
          setPhone(mobile);
          const view = renderSignedIn(Page, { path: '/admin/email', api: api() });
          await prepare?.();
          expect(await violations(view)).toEqual([]);
        });
});
