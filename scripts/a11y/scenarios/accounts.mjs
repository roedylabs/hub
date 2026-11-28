// Account › Accounts, the return from Stalwart's sign-in and "No account yet" in the Chromium audit
// (scripts/a11y/audit.mjs): the list with a default account and one that needs a new sign-in, the form for adding
// an account, one account's page, the return while the hub connects the account and with an error, and "No
// account yet" for an app.
export default ({ mock }) => {
  const SERVERS = {
    servers: [
      {
        id: 's1',
        label: 'Mail',
        url: 'https://mail.example.com',
        host: 'mail.example.com',
        domains: ['example.com'],
        sign_in: true,
        reach: 'ok',
        cors: 'ok',
        oauth: 'ok',
      },
    ],
    allow_custom_servers: true,
  };
  const ALEX = {
    id: 'a1',
    email: 'alex@example.com',
    username: 'alex@example.com',
    url: 'https://mail.example.com',
    server: { id: 's1', label: 'Mail' },
    auth: 'oauth',
    session_url: 'https://mail.example.com/jmap/session',
    capabilities: [
      'urn:ietf:params:jmap:calendars',
      'urn:ietf:params:jmap:contacts',
      'urn:ietf:params:jmap:core',
      'urn:ietf:params:jmap:mail',
    ],
    default: true,
    color: 1,
    state: 'ok',
    quota: { used: 1.3 * 1024 ** 3, total: 5 * 1024 ** 3, checked_at: '2026-11-28T09:00:00.000Z' },
    created_at: '2026-11-20T08:00:00.000Z',
  };
  const TEAM = {
    ...ALEX,
    id: 'a2',
    email: 'team@example.com',
    username: 'team@example.com',
    default: false,
    color: 2,
    state: 'needs_sign_in',
    quota: null,
  };
  const at = (accounts, extra = {}) =>
    mock({
      signedIn: true,
      routes: {
        'GET stalwart/accounts': { body: { accounts } },
        'GET stalwart/servers': { body: SERVERS },
        'GET apps': {
          body: {
            apps: [
              {
                slug: 'mail',
                name: 'Mail',
                description: 'Email',
                url: '/mail/',
                icon: '/api/v1/apps/mail/icon.svg',
                status: 'online',
              },
            ],
          },
        },
        ...extra,
      },
    });
  const h1 = (page) => page.locator('h1').first().waitFor();
  return [
    {
      id: 'accounts list',
      path: '/account/accounts',
      opts: at([ALEX, TEAM]),
      prepare: (page) => page.getByText('team@example.com').first().waitFor(),
    },
    {
      id: 'accounts empty',
      path: '/account/accounts',
      opts: at([]),
      prepare: (page) => page.getByText('No account connected yet.').first().waitFor(),
    },
    {
      id: 'accounts new',
      path: '/account/accounts/new',
      opts: at([ALEX]),
      prepare: async (page) => {
        await page.getByRole('button', { name: 'Continue to sign-in' }).first().waitFor();
        await page.getByLabel('Email address').first().fill('sam@example.com');
      },
    },
    {
      id: 'account detail',
      path: '/account/accounts/a1',
      opts: at([ALEX, TEAM]),
      prepare: (page) => page.getByRole('heading', { level: 2, name: 'Storage' }).waitFor(),
    },
    {
      id: 'account needs sign-in',
      path: '/account/accounts/a2',
      opts: at([ALEX, TEAM]),
      prepare: (page) => page.getByRole('heading', { level: 2, name: 'Storage' }).waitFor(),
    },
    {
      id: 'callback busy',
      path: '/oauth/callback?state=s&code=c',
      opts: at([ALEX], { 'POST stalwart/sign-in/complete': () => new Promise(() => {}) }),
      prepare: (page) => page.getByText('Finishing sign-in').first().waitFor(),
    },
    {
      id: 'callback error',
      path: '/oauth/callback?state=s&error=access_denied',
      opts: at([ALEX], {
        'POST stalwart/sign-in/complete': {
          status: 422,
          body: { error: 'sign_in_failed', reason: 'access_denied', return_to: '/account/accounts' },
        },
      }),
      prepare: (page) => page.getByRole('button', { name: 'Back' }).first().waitFor(),
    },
    {
      id: 'no account',
      path: '/account/no-account?next=%2Fmail%2F',
      opts: at([]),
      prepare: h1,
    },
  ];
};
