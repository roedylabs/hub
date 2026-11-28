// Administration › Email in the Chromium audit (scripts/a11y/audit.mjs): load failed, connected, not set up,
// no connection, token sign-in with unsaved changes, field errors, and saving with "Checking …". The server
// comes from the Stalwart registry (`admin/stalwart/servers`).
export default ({ mock }) => {
  const STORED = {
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
    checked_at: '2026-11-26T10:00:00Z',
    identities: [{ id: 'i1', name: 'Example Org', email: 'noreply@example.com' }],
  };
  const NONE = {
    ...STORED,
    server_id: null,
    user: '',
    has_secret: false,
    identity_id: null,
    status: 'none',
    checked_at: null,
    identities: [],
  };
  const path = '/admin/email';
  const SERVERS = {
    servers: [{ id: 's1', label: 'Mail', url: 'https://mail.example.com', host: 'mail.example.com' }],
    redirect_uri: 'https://app.example.com/oauth/callback',
    system_mail_server: 's1',
  };
  const at = (mail, extra = {}) =>
    mock({
      signedIn: true,
      routes: { 'GET admin/system-mail': mail, 'GET admin/stalwart/servers': { body: SERVERS }, ...extra },
    });
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
        // The access token field in the middle: at the top the bar "Unsaved changes" would cover the server's
        // pick list (a target, unlike the address field it replaced).
        await field(page, 'Access token').evaluate((el) => el.scrollIntoView({ block: 'center' }));
      },
    },
    {
      id: 'email field errors',
      path,
      opts: at({ body: NONE }),
      prepare: async (page) => {
        await ready(page);
        await field(page, 'Username').fill('no reply');
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
