// Administration › Email in the Chromium audit (scripts/a11y/audit.mjs): load failed, connected, not set up,
// no connection, token sign-in with unsaved changes, field errors, and saving with "Checking …".
export default ({ mock }) => {
  const STORED = {
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
    checked_at: '2026-11-26T10:00:00Z',
    identities: [{ id: 'i1', name: 'Example Org', email: 'noreply@example.com' }],
  };
  const NONE = {
    ...STORED,
    url: null,
    user: '',
    has_secret: false,
    identity_id: null,
    status: 'none',
    checked_at: null,
    identities: [],
  };
  const path = '/admin/email';
  const at = (mail, extra = {}) => mock({ signedIn: true, routes: { 'GET admin/system-mail': mail, ...extra } });
  const ready = (page) => page.getByRole('heading', { level: 2, name: 'Status' }).waitFor();
  const field = (page, name) => page.getByLabel(name, { exact: true });
  return [
    {
      id: 'email load failed',
      path,
      opts: at({ status: 500, body: { error: 'internal' } }),
      prepare: (page) => page.getByRole('button', { name: 'Try again' }).waitFor(),
    },
    { id: 'email connected', path, opts: at({ body: STORED }), prepare: ready },
    { id: 'email not set up', path, opts: at({ body: NONE }), prepare: ready },
    {
      id: 'email no connection',
      path,
      opts: at({ body: { ...STORED, status: 'err', reason: 'auth_failed' } }),
      prepare: ready,
    },
    {
      id: 'email token unsaved',
      path,
      opts: at({ body: STORED }),
      prepare: async (page) => {
        await ready(page);
        await page.getByRole('button', { name: 'Token', exact: true }).dispatchEvent('click');
        await field(page, 'Access token').waitFor();
      },
    },
    {
      id: 'email field errors',
      path,
      opts: at({ body: STORED }),
      prepare: async (page) => {
        await ready(page);
        await field(page, 'Server address').fill('mail.example.com');
        await field(page, 'Reply-to address').fill('no-at');
        await page.getByRole('alert').first().waitFor();
      },
    },
    {
      id: 'email saving',
      path,
      opts: at({ body: STORED }, { 'PUT admin/system-mail': () => new Promise(() => {}) }),
      prepare: async (page) => {
        await ready(page);
        await field(page, 'Password').fill('new-password');
        await page
          .getByRole('region', { name: 'Unsaved changes' })
          .getByRole('button', { name: 'Save' })
          .dispatchEvent('click');
        await page.getByText('Checking …').first().waitFor();
      },
    },
  ];
};
