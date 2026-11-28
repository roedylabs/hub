/**
 * axe-core over Administration › Email in every state (loading, load failed,
 * connected, not set up, no server in the registry, no connection, token
 * sign-in with unsaved changes, field errors, server pick list open, saving),
 * desktop and mobile, light and dark. Colour contrast
 * is left to the browser audit (scripts/a11y/audit.mjs).
 */
import { fireEvent, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { NO_MAIL, type MailServer, type SystemMail } from '#lib/system-mail.svelte.ts';
import { violations } from '@roedy/ui/testing';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

afterEach(() => {
  resetSignedIn();
  delete document.documentElement.dataset.mode;
});

const SERVERS: MailServer[] = [
  { id: 's1', label: 'Example mail', url: 'https://mail.example.com', host: 'mail.example.com' },
  { id: 's2', label: 'Other mail', url: 'https://other.example.com/', host: 'other.example.com' },
];
const STORED: SystemMail = {
  server_id: 's1',
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
/** Account and registry. */
const routes = (mail: SystemMail, servers: MailServer[] = SERVERS): ApiRoutes => ({
  'GET admin/system-mail': [{ body: mail }],
  'GET admin/stalwart/servers': [
    { body: { servers, redirect_uri: 'https://hub.example.com/cb', system_mail_server: null } },
  ],
});

/** States; `api` builds fresh queues per test (`mockApi` consumes them). */
const STATES: Record<string, { api: () => ApiRoutes; prepare?: () => Promise<unknown> }> = {
  loading: { api: () => ({ 'GET admin/system-mail': [never] }) },
  failed: {
    api: () => ({ 'GET admin/system-mail': [{ status: 500, body: { error: 'internal' } }] }),
    prepare: () => vi.waitFor(() => expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument()),
  },
  connected: { api: () => routes(STORED), prepare: ready },
  'not set up': { api: () => routes(NO_MAIL), prepare: ready },
  'no server': {
    api: () => routes(NO_MAIL, []),
    prepare: async () => {
      await ready();
      await fireEvent.input(field('Username'), { target: { value: 'noreply@example.com' } });
      await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    },
  },
  'no connection': { api: () => routes({ ...STORED, status: 'err', reason: 'auth_failed' }), prepare: ready },
  'token, unsaved': {
    api: () => routes(STORED),
    prepare: async () => {
      await ready();
      await fireEvent.click(screen.getByRole('button', { name: 'Token' }));
    },
  },
  'field errors': {
    api: () => routes({ ...STORED, server_id: null }),
    prepare: async () => {
      await ready();
      await fireEvent.input(field('Username'), { target: { value: 'no reply' } });
      await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
      await fireEvent.input(field('Reply-to address'), { target: { value: 'no-at' } });
      await fireEvent.input(field('Sender name'), { target: { value: 'x'.repeat(101) } });
    },
  },
  'pick list open': {
    api: () => routes(STORED),
    prepare: async () => {
      await ready();
      // A combobox on the desktop, a button for the sheet on mobile.
      await fireEvent.click(screen.getByText('Example mail').closest('button') as HTMLElement);
    },
  },
  saving: {
    api: () => ({ ...routes(STORED), 'PUT admin/system-mail': [never] }),
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
