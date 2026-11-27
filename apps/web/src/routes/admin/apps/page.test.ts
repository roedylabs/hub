import { fireEvent, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ui } from '#lib/state/ui.svelte.ts';
import { offlineApp, pairedApp, PAIRING, pendingApp } from '#lib/test/apps.ts';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

const nav = vi.hoisted(() => ({ goto: vi.fn(() => Promise.resolve()) }));
vi.mock('$app/navigation', () => ({ goto: nav.goto }));

const APPS = [pairedApp(), pendingApp('2099-01-01T00:00:00Z'), offlineApp()];

afterEach(() => {
  resetSignedIn();
  nav.goto.mockClear();
});

function apps(routes: ApiRoutes = {}) {
  return renderSignedIn(Page, {
    path: '/admin/apps',
    api: { 'GET admin/apps': Array.from({ length: 4 }, () => ({ body: { apps: APPS } })), ...routes },
  });
}

describe('Administration › Apps', () => {
  it('lists the apps in their order with status and users, each opening its page', async () => {
    const view = apps();
    expect(view.getByRole('heading', { level: 1, name: 'Apps' })).toBeInTheDocument();
    const list = await view.findByRole('list', { name: 'Apps in the order of the home page' });
    const rows = within(list).getAllByRole('listitem');
    const text = (r: HTMLElement, sel: string) => r.querySelector(sel)?.textContent?.trim();
    expect(rows.map((r) => [text(r, '.name'), text(r, '.sub'), text(r, '.st')])).toEqual([
      ['Mail', '/mail/ · 3 users', 'Online'],
      ['wiki', '/wiki/', 'Waiting for pairing'],
      ['Notes', '/notes/ · 1 user', 'Offline'],
    ]);
    // The failing event delivery of "Notes" is marked.
    expect(within(rows[2] as HTMLElement).getByRole('img', { name: /Events can’t be delivered/ })).toBeInTheDocument();
    await fireEvent.click(within(rows[1] as HTMLElement).getByRole('button', { name: /Waiting for pairing/ }));
    expect(nav.goto).toHaveBeenCalledWith('/admin/apps/wiki');
  });

  it('reorders with the buttons, keeps the focus and announces the position', async () => {
    const view = apps({ 'PATCH admin/apps': [{ body: { apps: APPS } }] });
    const up = await view.findByRole('button', { name: 'Move Notes up' });
    expect(view.getByRole('button', { name: 'Move Mail up' })).toHaveAttribute('aria-disabled', 'true');
    up.focus();
    await fireEvent.click(up);
    await vi.waitFor(() =>
      expect(view.getAllByRole('listitem').map((r) => r.querySelector('.name')?.textContent)).toEqual([
        'Mail',
        'Notes',
        'wiki',
      ]),
    );
    expect(view.getByRole('status')).toHaveTextContent('Notes moved to position 2 of 3');
    await vi.waitFor(() => expect(view.getByRole('button', { name: 'Move Notes up' })).toHaveFocus());
    await vi.waitFor(() => expect(view.calls.some((c) => c.method === 'PATCH')).toBe(true));
    // The last one cannot move down.
    await fireEvent.click(view.getByRole('button', { name: 'Move wiki down' }));
    expect(view.calls.filter((c) => c.method === 'PATCH')).toHaveLength(1);
  });

  it('adds an app with errors at the fields and shows the pairing code once', async () => {
    const view = apps({
      'POST admin/apps': [
        { status: 422, body: { error: 'invalid', fields: { slug: { code: 'taken' } } } },
        { status: 201, body: { app: pendingApp('2099-01-01T00:00:00Z', { slug: 'chat' }), pairing: PAIRING } },
      ],
    });
    await fireEvent.click(await view.findByRole('button', { name: 'Add app' }));
    const form = view.getByRole('form', { name: 'Add app' });
    await fireEvent.click(within(form).getByRole('button', { name: 'Add' }));
    const slug = within(form).getByLabelText('Address');
    expect(slug).toHaveAttribute('aria-invalid', 'true');
    expect(slug).toHaveAccessibleDescription('Enter an address.');
    expect(within(form).getByLabelText('Internal address')).toHaveAccessibleDescription(
      'Enter the app’s internal address.',
    );
    await fireEvent.input(slug, { target: { value: 'mail' } });
    expect(slug).toHaveAccessibleDescription('The path of the app in the hub, e.g. /mail/.');
    await fireEvent.input(within(form).getByLabelText('Internal address'), {
      target: { value: 'http://chat.internal:4000' },
    });
    await fireEvent.click(within(form).getByRole('button', { name: 'Add' }));
    expect(await within(form).findByRole('alert')).toHaveTextContent('Another app already uses this address.');
    await fireEvent.input(slug, { target: { value: 'chat' } });
    await fireEvent.keyDown(slug, { key: 'Enter' });
    const code = await view.findByRole('region', { name: 'Pairing code for chat' });
    expect(within(code).getByLabelText('Pairing code')).toHaveTextContent(PAIRING.display);
    expect(within(code).getByRole('heading', { name: 'Pairing code for chat' })).toHaveFocus();
    await fireEvent.click(within(code).getByRole('button', { name: 'Close pairing code' }));
    expect(view.queryByRole('region', { name: 'Pairing code for chat' })).toBeNull();
  });

  it('closes the form with Escape and "Cancel"', async () => {
    const view = apps();
    await fireEvent.click(await view.findByRole('button', { name: 'Add app' }));
    await fireEvent.keyDown(view.getByLabelText('Address'), { key: 'Escape' });
    expect(view.queryByRole('form')).toBeNull();
    await fireEvent.click(view.getByRole('button', { name: 'Add app' }));
    await fireEvent.click(view.getByRole('button', { name: 'Cancel' }));
    expect(view.queryByRole('form')).toBeNull();
  });

  it('shows the empty state and the failure card', async () => {
    const empty = renderSignedIn(Page, { api: { 'GET admin/apps': [{ body: { apps: [] } }] } });
    expect(await empty.findByText('No apps yet')).toBeInTheDocument();
    empty.unmount();
    setPhone(true);
    const failed = renderSignedIn(Page, { api: { 'GET admin/apps': [{ status: 500, body: { error: 'internal' } }] } });
    expect(await failed.findByRole('alert')).toHaveTextContent('Couldn’t load apps');
    expect(ui.isMobile).toBe(true);
  });
});
