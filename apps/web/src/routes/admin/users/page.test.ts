// Taken over from Roedy Mail (`apps/web/src/routes/admin/users/page.test.ts`) without undo, the kept list and the
// shortcuts of the shell's search; with the details of the expanded row.
import { createEvent, fireEvent, render, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mockApi } from '#lib/state/fetch-mock.ts';
import { ui } from '#lib/state/ui.svelte.ts';
import { pairedApp, pendingApp } from '#lib/test/apps.ts';
import { renderSignedIn, resetSignedIn, setPhone, TEST_USER, type ApiRoutes } from '#lib/test/signed-in.ts';
import type { ListedUser } from '#lib/users.svelte.ts';
import Page from './+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));

const NOW = Date.parse('2026-10-26T12:00:00Z');
const before = (hours: number) => new Date(NOW - hours * 3600_000).toISOString();
const base = { mfa: false, created_at: before(500), last_seen_at: before(2) };
const USERS: ListedUser[] = [
  { ...base, id: TEST_USER.id, status: 'active', name: TEST_USER.name, email: TEST_USER.email, role: 'admin' },
  { ...base, id: 'u1', status: 'active', name: 'Alice Brown', email: 'alice@example.com', role: 'admin', mfa: true },
  { ...base, id: 'u3', status: 'locked', name: 'Sophie Taylor', email: 'sophie@example.com', role: 'user' },
  {
    id: 'i2',
    status: 'invited',
    name: null,
    email: 'paul@example.com',
    role: 'user',
    created_at: before(1),
    mail_sent_at: null,
    expires_at: before(-100),
    expired: false,
    requested_at: null,
  },
];
const many = (users: ListedUser[]) => Array.from({ length: 20 }, () => ({ body: { users } }));

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
  nav.goto.mockClear();
  resetSignedIn();
});

function renderPage(routes: ApiRoutes = {}, mail = 'ok') {
  return renderSignedIn(Page, {
    path: '/admin/users',
    api: { 'GET users': many(USERS), 'GET admin/system-mail': [{ body: { status: mail } }], ...routes },
  });
}

describe('Users page', () => {
  it('shows heading, list with columns and "Not sent"', async () => {
    const view = renderPage();
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Users');
    expect(view.getByText('Create, edit, and suspend accounts.')).toBeInTheDocument();
    expect(await view.findByText('Alice Brown')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Sort by Name' })).toBeInTheDocument();
    expect(view.getByText('You')).toBeInTheDocument();
    expect(view.getByText('Not sent')).toBeInTheDocument();
    expect(view.getByRole('img', { name: 'Multi-factor enabled' })).toBeInTheDocument();
    expect(view.getByRole('checkbox', { name: 'You can’t select your own account' })).toBeInTheDocument();
    expect(view.queryByRole('note')).toBeNull();
    const sophie = view.getByRole('button', { name: /^Sophie Taylor/, expanded: false });
    const av = sophie.querySelector('.av');
    expect(av).toHaveClass('locked');
    expect(av).toHaveAttribute('aria-hidden', 'true');
  });

  it('filters via search and resets the filter', async () => {
    const view = renderPage();
    await view.findByText('Alice Brown');
    await fireEvent.input(view.getByLabelText('Search users'), { target: { value: 'xyz' } });
    expect(view.getByText('No results for “xyz”')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Clear filter' }));
    expect(view.getByText('Alice Brown')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: /^Suspended/ }));
    expect(view.queryByText('Alice Brown')).toBeNull();
    expect(view.getByRole('button', { name: /^Suspended/ })).toHaveAttribute('aria-pressed', 'true');
  });

  it('sends invitations: chips, cleared field, sending and toast', async () => {
    const view = renderPage({
      'POST invites': [
        {
          status: 201,
          body: {
            invites: [
              { id: 'n1', email: 'eve@example.com', sent: true },
              { id: 'n2', email: 'tim@example.com', sent: true },
            ],
            system_mail: true,
          },
        },
      ],
    });
    await view.findByText('Alice Brown');
    await fireEvent.click(view.getByRole('button', { name: 'Invite' }));
    expect(view.getByText('Invite people')).toBeInTheDocument();
    const field = view.getByLabelText('Email addresses') as HTMLInputElement;
    await vi.waitFor(() => expect(field).toHaveFocus());
    await fireEvent.input(field, { target: { value: 'eve@example.com, tim@example.com ' } });
    expect(field.value).toBe('');
    expect(view.getByRole('button', { name: 'Remove eve@example.com' })).toBeInTheDocument();
    expect(view.getByRole('textbox', { name: 'Name' })).toBeDisabled();
    await fireEvent.click(view.getByRole('button', { name: 'Send 2 invitations' }));
    await vi.waitFor(() => expect(ui.toast?.text).toBe('2 invitations sent'));
    expect(view.queryByText('Invite people')).toBeNull();
  });

  it('reports a name over 100 characters instead of cutting it off', async () => {
    const view = renderPage();
    await view.findByText('Alice Brown');
    await fireEvent.click(view.getByRole('button', { name: 'Invite' }));
    await fireEvent.input(view.getByLabelText('Email addresses'), { target: { value: 'eve@example.com' } });
    const name = view.getByRole('textbox', { name: 'Name' }) as HTMLInputElement;
    expect(name).not.toHaveAttribute('maxlength');
    await fireEvent.input(name, { target: { value: 'E'.repeat(101) } });
    expect(view.getByRole('alert')).toHaveTextContent('The name is too long.');
    expect(view.getByRole('button', { name: 'Send invitation' })).toHaveAttribute('aria-disabled', 'true');
    await fireEvent.input(name, { target: { value: 'E'.repeat(100) } });
    expect(view.getByRole('alert')).toHaveTextContent('');
    expect(view.getByRole('button', { name: 'Send invitation' })).toHaveAttribute('aria-disabled', 'false');
  });

  it('marks taken addresses and closes the form with Escape', async () => {
    const view = renderPage();
    await view.findByText('Alice Brown');
    await fireEvent.click(view.getByRole('button', { name: 'Invite' }));
    const field = view.getByLabelText('Email addresses');
    await vi.waitFor(() => expect(field).toHaveFocus());
    await fireEvent.input(field, { target: { value: 'alice@example.com' } });
    await fireEvent.keyDown(field, { key: 'Enter' });
    expect(view.getByRole('alert')).toHaveTextContent('alice@example.com already belongs to an account.');
    expect(view.getByRole('button', { name: /Send invitation/ })).toHaveAttribute('aria-disabled', 'true');
    await fireEvent.keyDown(field, { key: 'Escape' });
    expect(view.queryByText('Invite people')).toBeNull();
    // The focused field is gone: focus returns to "Invite", also after "Cancel".
    const invite = view.getByRole('button', { name: 'Invite' });
    await vi.waitFor(() => expect(invite).toHaveFocus());
    await fireEvent.click(invite);
    await vi.waitFor(() => expect(view.getByLabelText('Email addresses')).toHaveFocus());
    await fireEvent.click(view.getByRole('button', { name: 'Cancel' }));
    await vi.waitFor(() => expect(view.getByRole('button', { name: 'Invite' })).toHaveFocus());
  });

  it('shows the notice with "Set up email" and "Create invitation" without system mail', async () => {
    const view = renderPage({}, 'none');
    await view.findByText('Alice Brown');
    await vi.waitFor(() => expect(view.getByRole('note')).toHaveTextContent('Invitations aren’t sent by email'));
    await fireEvent.click(view.getByRole('button', { name: /Set up email/ }));
    expect(nav.goto).toHaveBeenCalledWith('/admin/email');
    await fireEvent.click(view.getByRole('button', { name: 'Invite' }));
    expect(
      view.getByText(
        'Separate addresses with commas, spaces, or Enter. You can then copy the invitation link from the list.',
      ),
    ).toBeInTheDocument();
    await fireEvent.input(view.getByLabelText('Email addresses'), { target: { value: 'eve@example.com' } });
    expect(view.getByRole('button', { name: /Create invitation/ })).toBeInTheDocument();
  });

  it('shows the result with "Copy link" when a mail did not go out', async () => {
    const eve: ListedUser = { ...USERS[3]!, id: 'n1', email: 'eve@example.com' };
    const view = renderPage({
      'GET users': [{ body: { users: USERS } }, ...many([...USERS, eve])],
      'POST invites': [
        { status: 201, body: { invites: [{ id: 'n1', email: 'eve@example.com', sent: false }], system_mail: true } },
      ],
    });
    await view.findByText('Alice Brown');
    await fireEvent.click(view.getByRole('button', { name: 'Invite' }));
    await fireEvent.input(view.getByLabelText('Email addresses'), { target: { value: 'eve@example.com' } });
    await fireEvent.click(view.getByRole('button', { name: 'Send invitation' }));
    const result = await view.findByText('Invitation couldn’t be sent');
    const box = result.closest('[role="alert"]') as HTMLElement;
    expect(within(box).getByText('eve@example.com')).toBeInTheDocument();
    expect(within(box).getByRole('button', { name: 'Copy link' })).toBeInTheDocument();
    await fireEvent.click(within(box).getByRole('button', { name: 'Close notice' }));
    expect(view.queryByText('Invitation couldn’t be sent')).toBeNull();
  });

  it('expands a row with devices and recovery codes and asks before locking', async () => {
    const locked = USERS.map((u) => (u.id === 'u1' ? { ...u, status: 'locked' as const } : u));
    const view = renderPage({
      'GET users': [{ body: { users: USERS } }, ...many(locked)],
      'GET users/u1': [
        {
          body: {
            user: {
              ...USERS[1],
              sessions: [{ created_at: before(30), last_seen_at: before(2) }],
              recovery_codes: { unused: 9, total: 10 },
            },
          },
        },
        { body: { user: { ...USERS[1], status: 'locked', sessions: [], recovery_codes: { unused: 9, total: 10 } } } },
      ],
      'POST users/u1/lock': [{ body: { user: { ...USERS[1], status: 'locked' } } }],
    });
    await view.findByText('Alice Brown');
    await fireEvent.click(view.getByRole('button', { name: /Alice Brown/, expanded: false }));
    const radios = view.getByRole('radiogroup', { name: 'Role' });
    expect(within(radios).getByRole('radio', { name: /Administrator/ })).toHaveAttribute('aria-checked', 'true');
    expect(await view.findByText('Signed in on 1 device · active 2 hours ago')).toBeInTheDocument();
    expect(view.getByText('9 of 10 recovery codes left')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Suspend' }));
    expect(ui.confirm?.title).toBe('Suspend Alice Brown?');
    ui.confirmOk();
    await vi.waitFor(() => expect(view.getAllByText('Suspended').length).toBeGreaterThan(1));
    await vi.waitFor(() => expect(ui.toast).toMatchObject({ text: 'Alice Brown suspended', error: false }));
    expect(view.getByRole('button', { name: 'Reactivate' })).toBeInTheDocument();
    expect(await view.findByText('Not signed in on any device')).toBeInTheDocument();
  });

  it('offers "Try again" when the details fail', async () => {
    const view = renderPage({
      'GET users/u1': [
        { status: 500, body: { error: 'internal' } },
        { body: { user: { ...USERS[1], sessions: [], recovery_codes: null } } },
      ],
    });
    await view.findByText('Alice Brown');
    await fireEvent.click(view.getByRole('button', { name: /Alice Brown/, expanded: false }));
    expect(await view.findByText('Couldn’t load devices and recovery codes.')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Try again' }));
    expect(await view.findByText('Not signed in on any device')).toBeInTheDocument();
    expect(view.getByText('No recovery codes')).toBeInTheDocument();
  });

  it('leads to Account › Profile for the own account', async () => {
    const view = renderPage({
      [`GET users/${TEST_USER.id}`]: [{ body: { user: { ...USERS[0], sessions: [], recovery_codes: null } } }],
    });
    await view.findByText('Alice Brown');
    await fireEvent.click(view.getByRole('button', { name: /Alex Example/, expanded: false }));
    expect(view.getByText('Only another administrator can change your own role.')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Edit your account' }));
    expect(nav.goto).toHaveBeenCalledWith('/account/profile');
  });

  it('selects and changes the role via the pick list', async () => {
    const view = renderPage({ 'POST users/bulk': [{ body: { changed: ['u3', 'i2'] } }] });
    await view.findByText('Alice Brown');
    await fireEvent.click(view.getByRole('checkbox', { name: 'Select Sophie Taylor' }));
    await fireEvent.click(view.getByRole('checkbox', { name: 'Select paul@example.com' }));
    expect(view.getByRole('toolbar', { name: 'Edit selection' })).toHaveTextContent('2 selected');
    expect(view.queryByRole('group', { name: 'Filter by status' })).toBeNull();
    const roleBtn = view.getByRole('combobox', { name: 'Change role' });
    roleBtn.focus();
    await fireEvent.click(roleBtn);
    const list = view.getByRole('listbox', { name: 'Role for 2 people' });
    const admin = within(list).getByRole('option', { name: /Administrator/ });
    const down = createEvent.mouseDown(admin);
    await fireEvent(admin, down);
    expect(down.defaultPrevented).toBe(true);
    await fireEvent.click(admin);
    expect(view.queryByRole('listbox')).toBeNull();
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Role changed for 2 people'));
    await fireEvent.click(view.getByRole('button', { name: 'Clear selection' }));
    expect(view.getByRole('group', { name: 'Filter by status' })).toBeInTheDocument();
  });

  it('names the list and the highlighted role on "Change role" while open', async () => {
    const view = renderPage();
    await view.findByText('Alice Brown');
    await fireEvent.click(view.getByRole('checkbox', { name: 'Select Sophie Taylor' }));
    const roleBtn = view.getByRole('combobox', { name: 'Change role' });
    expect(roleBtn).toHaveAttribute('aria-expanded', 'false');
    expect(roleBtn).not.toHaveAttribute('aria-controls');
    await fireEvent.keyDown(roleBtn, { key: 'ArrowDown' });
    const list = view.getByRole('listbox', { name: 'Role for 1 person' });
    expect(roleBtn).toHaveAttribute('aria-controls', list.id);
    const [admin, user] = within(list).getAllByRole('option');
    expect(user).toHaveAttribute('aria-selected', 'true');
    expect(roleBtn).toHaveAttribute('aria-activedescendant', user!.id);
    await fireEvent.keyDown(roleBtn, { key: 'ArrowDown' });
    expect(admin).toHaveClass('hi');
    expect(roleBtn).toHaveAttribute('aria-activedescendant', admin!.id);
  });

  it('shows avatar as selection, row with status and actions stacked on mobile', async () => {
    setPhone(true);
    const view = renderPage({
      'GET users/u1': [{ body: { user: { ...USERS[1], sessions: [], recovery_codes: null } } }],
    });
    await view.findByText('Alice Brown');
    expect(view.getByPlaceholderText('Name or email')).toBeInTheDocument();
    expect(view.getByText('· Not sent')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('checkbox', { name: 'Select Alice Brown' }));
    expect(view.getByRole('button', { name: 'Change role' })).toHaveAttribute('aria-haspopup', 'listbox');
    await fireEvent.click(view.getByRole('button', { name: 'Change role' }));
    // On mobile the layout shows the list as a sheet (`ui.pick`).
    expect(ui.menu).toBe('pick');
    expect(ui.pick?.().title).toBe('Role for 1 person');
    ui.closeMenu();
    await fireEvent.click(view.getByRole('button', { name: /Alice Brown/, expanded: false }));
    expect(view.getByText('alice@example.com · Last active 2 hours ago', { exact: false })).toBeInTheDocument();
  });

  it('offers "Extend" instead of "Copy link" for an expired invitation', async () => {
    const old: ListedUser = {
      ...base,
      id: 'i3',
      status: 'invited',
      name: 'Eve Moore',
      email: 'eve@example.com',
      role: 'user',
      mail_sent_at: before(200),
      expires_at: before(32),
      expired: true,
      requested_at: before(4),
    };
    const fresh = { ...old, expires_at: before(-168), expired: false };
    const view = renderPage({
      'GET users': [{ body: { users: [...USERS, old] } }, ...many([...USERS, fresh])],
      'POST invites/i3/renew': [{ body: { invite: fresh } }],
    });
    await view.findByText('Eve Moore');
    expect(view.getAllByText('Expired').length).toBeGreaterThan(0);
    expect(view.getByText('New invitation requested')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: /Eve Moore/, expanded: false }));
    expect(view.getByRole('alert')).toHaveTextContent('The invitee asked for a new invitation 4 hours ago.');
    expect(view.queryByRole('button', { name: 'Copy link' })).toBeNull();
    await fireEvent.click(view.getByRole('button', { name: 'Extend' }));
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Invitation extended until 2 November'));
    expect(view.getByText('Valid until 2 November')).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Copy link' })).toBeInTheDocument();
    expect(view.queryByRole('button', { name: 'Extend' })).toBeNull();
  });

  it('shows the skeleton until the first response, without search and list', async () => {
    const view = renderPage({ 'GET users': [() => new Promise(() => {})] });
    expect(view.getByRole('heading', { level: 1 })).toHaveTextContent('Users');
    expect(view.queryByText('Loading data …')).toBeNull();
    expect(await view.findByText('Loading data …')).toBeInTheDocument();
    expect(view.queryByRole('textbox')).toBeNull();
    expect(view.queryByRole('button')).toBeNull();
  });

  it('shows the card instead of the list when the first load fails, and reloads', async () => {
    const view = renderPage({
      'GET users': [{ status: 500, body: { error: 'internal' } }, { body: { users: USERS } }],
    });
    const card = await view.findByRole('alert');
    expect(card).toHaveTextContent('Couldn’t load users');
    expect(card).toHaveTextContent('That didn’t work. Please try again.');
    expect(view.queryByRole('textbox')).toBeNull();
    await fireEvent.click(view.getByRole('button', { name: 'Try again' }));
    expect(await view.findByText('Alice Brown')).toBeInTheDocument();
  });

  it('writes nothing once the page is left', async () => {
    let answer: (value: { body: unknown }) => void = () => {};
    mockApi({
      'GET users': [() => new Promise((resolve) => (answer = resolve))],
      'GET admin/system-mail': [{ body: { status: 'ok' } }],
    });
    const view = render(Page);
    view.unmount();
    answer({ body: { users: USERS } });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(ui.pick).toBeNull();
    expect(ui.toast).toBeNull();
  });
});

describe('Users page › app access', () => {
  const APPS = { body: { apps: [pairedApp(), pendingApp()] } };
  const withMail = USERS.map((u) => (u.id === 'u1' ? { ...u, apps: { mail: 'admin' } } : u));

  it('changes access per app in the expanded row with the radio group', async () => {
    const view = renderSignedIn(Page, {
      path: '/admin/users',
      api: {
        'GET users': many(withMail),
        'GET admin/system-mail': [{ body: { status: 'ok' } }],
        'GET admin/apps': [APPS, APPS],
        'GET users/u1': [{ body: { user: { ...withMail[1], sessions: [], recovery_codes: null } } }],
        'DELETE users/u1/apps/mail': [{ body: { user: { ...withMail[1], apps: {} } } }],
      },
    });
    await fireEvent.click(await view.findByRole('button', { name: /^Alice Brown/, expanded: false }));
    const group = await view.findByRole('radiogroup', { name: 'Access to Mail' });
    expect(
      within(group)
        .getAllByRole('radio')
        .map((r) => [r.textContent?.trim(), r.getAttribute('aria-checked')]),
    ).toEqual([
      ['No access', 'false'],
      ['User(default)', 'false'],
      ['Administrator', 'true'],
    ]);
    // The pending app offers no access.
    expect(view.queryByRole('radiogroup', { name: 'Access to wiki' })).toBeNull();
    expect(within(group).getByRole('radio', { name: /^User/ })).toHaveAccessibleDescription('Reads and sends email');
    await fireEvent.click(within(group).getByRole('radio', { name: 'No access' }));
    expect(within(group).getByRole('radio', { name: 'No access' })).toHaveAttribute('aria-checked', 'true');
    await vi.waitFor(() => expect(ui.toast?.text).toBe('Access to Mail removed'));
  });

  it('shows the access per app in the invitation form, with errors at the app', async () => {
    const view = renderSignedIn(Page, {
      path: '/admin/users',
      api: {
        'GET users': many(USERS),
        'GET admin/system-mail': [{ body: { status: 'ok' } }],
        'GET admin/apps': [APPS, APPS],
        'POST invites': [{ status: 422, body: { error: 'invalid', fields: { 'apps.mail': { code: 'app_pending' } } } }],
      },
    });
    await fireEvent.click(await view.findByRole('button', { name: 'Invite' }));
    const group = await view.findByRole('radiogroup', { name: 'Access to Mail' });
    expect(within(group).getByRole('radio', { name: /^User/ })).toHaveAttribute('aria-checked', 'true');
    await fireEvent.click(within(group).getByRole('radio', { name: 'Administrator' }));
    await fireEvent.input(view.getByLabelText('Email addresses'), { target: { value: 'tim@example.com' } });
    await fireEvent.click(view.getByRole('button', { name: 'Send invitation' }));
    const err = await view.findByText('This app isn’t paired yet.');
    expect(err).toHaveAttribute('role', 'alert');
    expect(view.getByRole('radiogroup', { name: 'Access to Mail' })).toHaveAccessibleDescription(
      'This app isn’t paired yet.',
    );
  });

  it('opens "App access" for the selection and applies it', async () => {
    const view = renderSignedIn(Page, {
      path: '/admin/users',
      api: {
        'GET users': many(USERS),
        'GET admin/system-mail': [{ body: { status: 'ok' } }],
        'GET admin/apps': [APPS],
        'POST users/bulk': [{ body: { changed: ['u3'] } }],
      },
    });
    await fireEvent.click(await view.findByRole('checkbox', { name: 'Select Sophie Taylor' }));
    const toggle = await view.findByRole('button', { name: 'App access' });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await fireEvent.click(toggle);
    const panel = view.getByRole('group', { name: 'App access for 1 person' });
    expect(
      within(within(panel).getByRole('radiogroup', { name: 'Access to Mail' })).getByRole('radio', {
        name: 'Unchanged',
      }),
    ).toHaveAttribute('aria-checked', 'true');
    expect(within(panel).getByRole('button', { name: 'Apply' })).toHaveAttribute('aria-disabled', 'true');
    // "Cancel" puts the focus back on the toggle.
    within(panel).getByRole('button', { name: 'Cancel' }).focus();
    await fireEvent.click(within(panel).getByRole('button', { name: 'Cancel' }));
    await vi.waitFor(() => expect(view.getByRole('button', { name: 'App access' })).toHaveFocus());
    await fireEvent.click(view.getByRole('button', { name: 'App access' }));
    const again = view.getByRole('group', { name: 'App access for 1 person' });
    const group = within(again).getByRole('radiogroup', { name: 'Access to Mail' });
    await fireEvent.click(within(group).getByRole('radio', { name: /^User/ }));
    await fireEvent.click(within(again).getByRole('button', { name: 'Apply' }));
    await vi.waitFor(() => expect(ui.toast?.text).toBe('App access changed for Sophie Taylor'));
    expect(view.queryByRole('group', { name: 'App access for 1 person' })).toBeNull();
  });
});
