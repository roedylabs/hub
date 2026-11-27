// The home page: tiles with an offline app and the hint for apps waiting for pairing (administrator), the empty
// state of a user and of an administrator, and a failed load.
import { ADMIN_APPS, HOME_APPS, ICONS } from '../apps-data.mjs';

export default ({ mock, SESSION }) => {
  const asUser = { ...SESSION, user: { ...SESSION.user, role: 'user' }, hub: { ...SESSION.hub, admin: null } };
  const tiles = (page) => page.getByRole('link', { name: 'Mail' }).first().waitFor();
  return [
    {
      id: 'home',
      path: '/',
      opts: mock({
        signedIn: true,
        routes: {
          ...ICONS,
          'GET apps': { body: { apps: HOME_APPS } },
          'GET admin/apps': { body: { apps: ADMIN_APPS } },
        },
      }),
      prepare: async (page) => {
        await tiles(page);
        await page.getByRole('note').first().waitFor();
      },
    },
    {
      id: 'home user',
      path: '/',
      opts: mock({
        signedIn: true,
        routes: { ...ICONS, 'GET session': { body: asUser }, 'GET apps': { body: { apps: HOME_APPS.slice(0, 2) } } },
      }),
      prepare: tiles,
    },
    {
      id: 'home empty user',
      path: '/',
      opts: mock({ signedIn: true, routes: { 'GET session': { body: asUser }, 'GET apps': { body: { apps: [] } } } }),
      prepare: (page) => page.getByText('No apps yet').first().waitFor(),
    },
    {
      id: 'home empty admin',
      path: '/',
      opts: mock({
        signedIn: true,
        routes: { 'GET apps': { body: { apps: [] } }, 'GET admin/apps': { body: { apps: [] } } },
      }),
      prepare: (page) => page.getByText('Add your first app').first().waitFor(),
    },
    {
      id: 'home load failed',
      path: '/',
      opts: mock({
        signedIn: true,
        routes: { 'GET apps': { status: 500, body: { error: 'internal' } }, 'GET admin/apps': { body: { apps: [] } } },
      }),
      prepare: (page) => page.getByText('Couldn’t load your apps').first().waitFor(),
    },
  ];
};
