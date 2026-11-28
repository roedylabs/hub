// Administration › Stalwart servers in the Chromium audit (scripts/a11y/audit.mjs): the list with a server of the
// system mail (diagnosis open after "Test connection"), the empty registry, and the dialog for adding a server.
export default ({ mock }) => {
  const path = '/admin/stalwart';
  const base = {
    client_id: null,
    has_secret: false,
    reach: 'unk',
    cors: 'unk',
    oauth: 'unk',
    tested_at: null,
    accounts: 0,
    created_at: '2026-11-01T00:00:00Z',
  };
  const MAIL = {
    ...base,
    id: 's1',
    label: 'Mail',
    url: 'https://mail.example.com',
    host: 'mail.example.com',
    domains: ['example.com', 'example.org'],
    client_id: 'roedy-hub-abc123',
    has_secret: true,
    reach: 'ok',
    cors: 'ok',
    oauth: 'ok',
    tested_at: '2026-11-28T09:00:00Z',
    accounts: 12,
  };
  const LAB = {
    ...base,
    id: 's2',
    label: 'Lab',
    url: 'https://jmap.lab.example.net',
    host: 'jmap.lab.example.net',
    domains: ['lab.example.net'],
    reach: 'ok',
    cors: 'err',
    oauth: 'unk',
    tested_at: '2026-11-28T08:00:00Z',
  };
  const list = (servers) => ({
    body: { servers, redirect_uri: 'https://app.example.com/oauth/callback', system_mail_server: 's1' },
  });
  const at = (servers, extra = {}) =>
    mock({
      signedIn: true,
      routes: {
        'GET admin/stalwart/servers': list(servers),
        'GET admin/stalwart/policy': { body: { allow_custom_servers: false } },
        ...extra,
      },
    });
  const ready = (page) => page.getByRole('heading', { level: 2, name: 'Policy' }).waitFor();
  return [
    { id: 'stalwart servers', path, opts: at([MAIL, LAB]), prepare: ready },
    { id: 'stalwart servers empty', path, opts: at([]), prepare: ready },
    {
      id: 'stalwart diagnosis',
      path,
      opts: at([MAIL, LAB], {
        'POST admin/stalwart/servers/s2/test': {
          body: { server: { ...LAB, tested_at: '2026-11-28T10:00:00Z' }, reasons: { cors: 'origin_not_allowed' } },
        },
      }),
      prepare: async (page) => {
        await ready(page);
        await page.getByRole('button', { name: 'Test connection' }).last().dispatchEvent('click');
        await page.getByRole('region').first().waitFor();
      },
    },
    {
      id: 'stalwart add dialog',
      path,
      opts: at([MAIL]),
      prepare: async (page) => {
        await ready(page);
        await page.getByRole('button', { name: 'Add server' }).first().dispatchEvent('click');
        await page.getByRole('dialog').waitFor();
      },
    },
  ];
};
