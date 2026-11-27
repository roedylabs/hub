// Administration › Apps: the list with a paired, an offline and a pending app, the form "Add app" with field
// errors, the pairing code after adding, the empty list; the page of a paired app with roles and who has access,
// of a pending app with a new pairing code, of an offline one, and of an unknown one.
import { ADMIN_APPS, ICONS, PAIRING } from '../apps-data.mjs';

export default ({ mock, button }) => {
  const [mail, notes, calendar] = ADMIN_APPS;
  const users = {
    users: [
      {
        id: '01JUSER',
        status: 'active',
        name: 'Alex Example',
        email: 'alex@example.com',
        role: 'admin',
        app_role: 'admin',
      },
      {
        id: 'u1',
        status: 'active',
        name: 'Jordan Hale',
        email: 'jordan@example.com',
        role: 'admin',
        app_role: 'admin',
      },
      { id: 'u2', status: 'active', name: 'Sam Rivera', email: 'sam@example.com', role: 'user', app_role: 'user' },
      { id: 'u3', status: 'locked', name: 'Robin Park', email: 'robin@example.com', role: 'user', app_role: 'user' },
      { id: 'u4', status: 'active', name: 'Quinn Lee', email: 'quinn@example.com', role: 'user', app_role: null },
    ],
    invites: [{ id: 'i1', status: 'invited', name: null, email: 'casey@example.com', role: 'user', app_role: 'user' }],
  };
  const api = (routes = {}) =>
    mock({
      signedIn: true,
      routes: {
        ...ICONS,
        'GET admin/apps': { body: { apps: ADMIN_APPS } },
        'GET admin/apps/mail': { body: { app: mail } },
        'GET admin/apps/mail/users': { body: users },
        'GET admin/apps/notes': { body: { app: notes } },
        'GET admin/apps/notes/users': { body: { users: users.users.slice(0, 2), invites: [] } },
        'GET admin/apps/calendar': { body: { app: calendar } },
        'POST admin/apps/calendar/pairing-code': { body: { app: calendar, pairing: PAIRING } },
        ...routes,
      },
    });
  const loaded = (page) => page.getByText('Waiting for pairing').first().waitFor();
  const openAdd = async (page) => {
    await loaded(page);
    await button(page, 'Add app');
    await page.getByLabel('Address').first().waitFor();
  };

  return [
    { id: 'apps list', path: '/admin/apps', opts: api(), prepare: loaded },
    {
      id: 'apps add with errors',
      path: '/admin/apps',
      opts: api(),
      prepare: async (page) => {
        await openAdd(page);
        await page.getByLabel('Address').first().fill('Admin');
        await page.getByLabel('Internal address').first().fill('mail.internal/x');
        await button(page, 'Add');
        await page.getByText('The hub uses this address itself. Choose another one.').first().waitFor();
      },
    },
    {
      id: 'apps pairing code',
      path: '/admin/apps',
      opts: api({
        'POST admin/apps': {
          status: 201,
          body: { app: { ...calendar, slug: 'wiki', id: 'app-wiki' }, pairing: PAIRING },
        },
      }),
      prepare: async (page) => {
        await openAdd(page);
        await page.getByLabel('Address').first().fill('wiki');
        await page.getByLabel('Internal address').first().fill('http://wiki.internal:3000');
        await button(page, 'Add');
        await page.getByText('Pairing code for wiki').first().waitFor();
      },
    },
    {
      id: 'apps empty',
      path: '/admin/apps',
      opts: api({ 'GET admin/apps': { body: { apps: [] } } }),
      prepare: (page) => page.getByText('No apps yet').first().waitFor(),
    },
    {
      id: 'app paired',
      path: '/admin/apps/mail',
      opts: api(),
      prepare: (page) => page.getByText('5 of 6 with access, including open invitations.').first().waitFor(),
    },
    {
      id: 'app offline',
      path: '/admin/apps/notes',
      opts: api(),
      prepare: (page) =>
        page
          .getByText(/Events can’t be delivered/)
          .first()
          .waitFor(),
    },
    {
      id: 'app pending with code',
      path: '/admin/apps/calendar',
      opts: api(),
      prepare: async (page) => {
        await button(page, 'New pairing code');
        await page.getByText('Pairing code for calendar').first().waitFor();
      },
    },
    {
      id: 'app not found',
      path: '/admin/apps/nope',
      opts: api(),
      prepare: (page) => page.getByText('It may have been removed in the meantime.').first().waitFor(),
    },
  ];
};
