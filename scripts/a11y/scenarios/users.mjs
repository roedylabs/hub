// Administration › Users: the list, an expanded user with devices, recovery codes and app access, an expired
// invitation the invitee asked to renew, the invitation form with marked addresses and app access, the result
// notice after inviting, the selection with "Change role" open and with "App access", no system mail, an empty
// search and a failed first load.
import { ADMIN_APPS, ICONS } from '../apps-data.mjs';

export default ({ mock, button, USER }) => {
  const at = (hoursAgo) => new Date(Date.now() - hoursAgo * 3_600_000).toISOString();
  const base = { mfa: false, created_at: at(500), last_seen_at: at(2) };
  const users = [
    { ...base, id: USER.id, status: 'active', name: USER.name, email: USER.email, username: 'alex', role: 'admin' },
    {
      ...base,
      id: 'u1',
      status: 'active',
      name: 'Jordan Hale',
      email: 'jordan@example.com',
      username: 'jordan',
      role: 'admin',
      mfa: true,
      apps: { mail: 'admin', notes: 'user' },
    },
    {
      ...base,
      id: 'u2',
      status: 'active',
      name: 'Sam Rivera',
      email: 'sam@example.com',
      username: 'sam',
      role: 'user',
      last_seen_at: at(30),
    },
    {
      ...base,
      id: 'u3',
      status: 'locked',
      name: 'Robin Park',
      email: 'robin@example.com',
      username: 'robin',
      role: 'user',
      last_seen_at: at(120),
    },
    {
      id: 'i1',
      status: 'invited',
      name: null,
      email: 'casey@example.com',
      role: 'user',
      created_at: at(1),
      invited_by: { id: USER.id, name: USER.name },
      mail_sent_at: null,
      expires_at: at(-100),
      expired: false,
      requested_at: null,
    },
    {
      id: 'i2',
      status: 'invited',
      name: 'Taylor Brooks',
      email: 'taylor@example.com',
      role: 'user',
      created_at: at(300),
      invited_by: { id: USER.id, name: USER.name },
      mail_sent_at: at(200),
      expires_at: at(30),
      expired: true,
      requested_at: at(4),
    },
  ];
  const details = {
    body: {
      user: {
        ...users[1],
        sessions: [
          { created_at: at(40), last_seen_at: at(2) },
          { created_at: at(10), last_seen_at: at(9) },
        ],
        recovery_codes: { unused: 8, total: 10 },
      },
    },
  };
  const path = '/admin/users';
  const api = (routes = {}, mail = 'ok') =>
    mock({
      signedIn: true,
      routes: {
        'GET users': { body: { users } },
        'GET admin/system-mail': { body: { status: mail } },
        'GET users/u1': details,
        'GET admin/apps': { body: { apps: ADMIN_APPS } },
        ...ICONS,
        ...routes,
      },
    });
  const loaded = (page) => page.getByText('Jordan Hale').first().waitFor();
  const expand = async (page, name) => {
    await loaded(page);
    await page.getByRole('button', { name, expanded: false }).first().dispatchEvent('click');
  };
  const openInvite = async (page) => {
    await loaded(page);
    await button(page, 'Invite');
    await page.getByLabel('Email addresses').first().waitFor();
  };

  return [
    { id: 'users list', path, opts: api(), prepare: loaded },
    {
      id: 'users expanded with details',
      path,
      opts: api(),
      prepare: async (page) => {
        await expand(page, /Jordan Hale/);
        await page.getByText('8 of 10 recovery codes left').first().waitFor();
        await page.getByRole('radiogroup', { name: 'Access to Mail' }).first().waitFor();
      },
    },
    {
      id: 'users details failed',
      path,
      opts: api({ 'GET users/u1': { status: 500, body: { error: 'internal' } } }),
      prepare: async (page) => {
        await expand(page, /Jordan Hale/);
        await page.getByText('Couldn’t load devices and recovery codes.').first().waitFor();
      },
    },
    {
      id: 'users expired invitation requested',
      path,
      opts: api(),
      prepare: (page) => expand(page, /Taylor Brooks/),
    },
    {
      id: 'users invitation form',
      path,
      opts: api(),
      prepare: async (page) => {
        await openInvite(page);
        await page.getByLabel('Email addresses').first().fill('jordan@example.com new@example.com broken@ ');
        await page.getByRole('radiogroup', { name: 'Access to Mail' }).first().waitFor();
      },
    },
    {
      id: 'users invitation result',
      path,
      opts: api({
        'POST invites': {
          status: 201,
          body: { invites: [{ id: 'i1', email: 'casey@example.com', sent: false }], system_mail: true },
        },
      }),
      prepare: async (page) => {
        await openInvite(page);
        await page.getByLabel('Email addresses').first().fill('quinn@example.com');
        await page.getByRole('button', { name: /^Send/ }).filter({ visible: true }).first().dispatchEvent('click');
        await page.getByText('Invitation couldn’t be sent').first().waitFor();
      },
    },
    {
      id: 'users selection change role',
      path,
      opts: api(),
      prepare: async (page) => {
        await loaded(page);
        await page.getByRole('checkbox', { name: 'Select Robin Park' }).first().dispatchEvent('click');
        await page
          .getByRole('combobox', { name: 'Change role' })
          .or(page.getByRole('button', { name: 'Change role' }))
          .first()
          .dispatchEvent('click');
        await page.getByText('Role for 1 person').first().waitFor();
      },
    },
    {
      id: 'users selection app access',
      path,
      opts: api(),
      prepare: async (page) => {
        await loaded(page);
        await page.getByRole('checkbox', { name: 'Select Robin Park' }).first().dispatchEvent('click');
        await page.getByRole('button', { name: 'App access' }).first().dispatchEvent('click');
        await page.getByText('App access for 1 person').first().waitFor();
      },
    },
    {
      id: 'users without system mail',
      path,
      opts: api({}, 'none'),
      prepare: async (page) => {
        await loaded(page);
        await page.getByRole('note').first().waitFor();
      },
    },
    {
      id: 'users empty search',
      path,
      opts: api(),
      prepare: async (page) => {
        await loaded(page);
        await page.getByLabel('Search users').first().fill('nobody');
        await page.getByText('No results for “nobody”').first().waitFor();
      },
    },
    {
      id: 'users load failed',
      path,
      opts: api({ 'GET users': { status: 500, body: { error: 'internal' } } }),
      prepare: (page) => page.getByText('Couldn’t load users').first().waitFor(),
    },
  ];
};
