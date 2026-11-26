// Administration › Audit log: entries by day with a warning and a further page, a search, the empty state, both
// parts failed, and the page in German.
export default ({ mock }) => {
  const at = (hoursAgo) => new Date(Date.now() - hoursAgo * 3_600_000).toISOString();
  const entries = [
    {
      id: 5,
      at: at(1),
      cat: 'login',
      icon: 'ti-login',
      who: 'Jordan Hale',
      act: ' signed in',
      meta: '192.0.2.x',
      warn: false,
    },
    {
      id: 4,
      at: at(2),
      cat: 'system',
      icon: 'ti-user-shield',
      who: 'System',
      act: ': administrator access reset',
      meta: 'alex@example.com',
      warn: true,
    },
    {
      id: 3,
      at: at(3),
      cat: 'settings',
      icon: 'ti-brush',
      who: 'Alex Example',
      act: ' changed the branding',
      meta: '',
      warn: false,
    },
    {
      id: 2,
      at: at(28),
      cat: 'apps',
      icon: 'ti-apps',
      who: 'Alex Example',
      act: ' opened an app',
      meta: '',
      warn: false,
    },
    {
      id: 1,
      at: at(80),
      cat: 'users',
      icon: 'ti-user-plus',
      who: 'Alex Example',
      act: ' invited Robin Lee',
      meta: '',
      warn: false,
    },
  ];
  const counts = { all: 5, login: 1, users: 1, apps: 1, settings: 1, system: 1 };
  const keep = { 'GET admin/audit/retention': { body: { days: 90 } } };
  const list = (body) => ({ ...keep, 'GET admin/audit': { body } });
  const full = list({ entries, counts, next: 1 });
  return [
    {
      id: 'audit',
      path: '/admin/audit-log',
      opts: mock({ signedIn: true, routes: full }),
      prepare: (page) => page.getByText('Jordan Hale').first().waitFor(),
    },
    {
      id: 'audit search',
      path: '/admin/audit-log',
      opts: mock({ signedIn: true, routes: full }),
      prepare: async (page) => {
        await page.getByText('Jordan Hale').first().waitFor();
        await page.getByRole('textbox', { name: 'Search audit log' }).fill('jordan');
        await page.getByRole('button', { name: 'Clear search' }).waitFor();
      },
    },
    {
      id: 'audit empty',
      path: '/admin/audit-log',
      opts: mock({
        signedIn: true,
        routes: list({
          entries: [],
          counts: { all: 0, login: 0, users: 0, apps: 0, settings: 0, system: 0 },
          next: null,
        }),
      }),
      prepare: (page) => page.getByText('No entries found').first().waitFor(),
    },
    {
      id: 'audit failed',
      path: '/admin/audit-log',
      opts: mock({
        signedIn: true,
        routes: {
          'GET admin/audit': { status: 500, body: { error: 'internal' } },
          'GET admin/audit/retention': { status: 500, body: { error: 'internal' } },
        },
      }),
      prepare: (page) => page.getByText('Couldn’t load the retention setting').first().waitFor(),
    },
    {
      id: 'audit german',
      path: '/admin/audit-log',
      opts: mock({
        signedIn: true,
        routes: {
          ...full,
          'GET settings': { body: { version: 1, settings: { lang: 'de', mode: null, accent: null } } },
        },
      }),
      prepare: (page) => page.getByRole('heading', { name: 'Aufbewahrung' }).first().waitFor(),
    },
  ];
};
