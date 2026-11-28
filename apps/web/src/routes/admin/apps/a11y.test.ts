/**
 * axe-core over Administration › Apps in every state (list with a paired,
 * a pending and an offline app, the form with field errors, the pairing
 * code, empty, skeleton, failure card), desktop and mobile, light and dark.
 * The colour contrast is left to the browser audit
 * (`scripts/a11y/scenarios/apps.mjs`).
 */
import { fireEvent, type RenderResult } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { offlineApp, pairedApp, PAIRING, pendingApp } from '#lib/test/apps.ts';
import { violations } from '@roedy/ui/testing';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

const APPS = [pairedApp(), pendingApp('2099-01-01T00:00:00Z'), offlineApp()];

afterEach(() => {
  resetSignedIn();
  delete document.documentElement.dataset.mode;
});

type View = RenderResult<never>;

async function apps(opts: { phone?: boolean; routes?: ApiRoutes; then?: (v: View) => Promise<unknown> } = {}) {
  setPhone(opts.phone ?? false);
  const view = renderSignedIn(Page, {
    path: '/admin/apps',
    api: { 'GET admin/apps': Array.from({ length: 3 }, () => ({ body: { apps: APPS } })), ...opts.routes },
  }) as unknown as View;
  await opts.then?.(view);
  return view;
}

const loaded = (view: View) => view.findByText('Notes');
const formWithErrors = async (view: View) => {
  await fireEvent.click(await view.findByRole('button', { name: 'Add app' }));
  await fireEvent.input(view.getByLabelText('Address'), { target: { value: '2x' } });
  await fireEvent.click(view.getByRole('button', { name: 'Add' }));
  await view.findAllByRole('alert');
};
const withCode = async (view: View) => {
  await fireEvent.click(await view.findByRole('button', { name: 'Add app' }));
  await fireEvent.input(view.getByLabelText('Address'), { target: { value: 'chat' } });
  await fireEvent.input(view.getByLabelText('Internal address'), { target: { value: 'http://chat.internal' } });
  await fireEvent.click(view.getByRole('button', { name: 'Add' }));
  await view.findByRole('region', { name: 'Pairing code for chat' });
};
const created = (): ApiRoutes => ({
  'POST admin/apps': [
    { status: 201, body: { app: pendingApp('2099-01-01T00:00:00Z', { slug: 'chat' }), pairing: PAIRING } },
  ],
});

const CASES: [string, () => Promise<View>][] = [
  ['list (desktop)', () => apps({ then: loaded })],
  ['list (mobile)', () => apps({ phone: true, then: loaded })],
  ['form with errors (desktop)', () => apps({ then: formWithErrors })],
  ['form with errors (mobile)', () => apps({ phone: true, then: formWithErrors })],
  ['pairing code (desktop)', () => apps({ routes: created(), then: withCode })],
  ['pairing code (mobile)', () => apps({ phone: true, routes: created(), then: withCode })],
  [
    'empty (desktop)',
    () => apps({ routes: { 'GET admin/apps': [{ body: { apps: [] } }] }, then: (v) => v.findByText('No apps yet') }),
  ],
  [
    'skeleton (desktop)',
    () =>
      apps({
        routes: { 'GET admin/apps': [() => new Promise(() => {})] },
        then: (v) => v.findByText('Loading data …'),
      }),
  ],
  [
    'failure card (mobile)',
    () =>
      apps({
        phone: true,
        routes: { 'GET admin/apps': [{ status: 500, body: { error: 'internal' } }] },
        then: (v) => v.findByRole('alert'),
      }),
  ],
];

describe('Administration › Apps accessibility (axe)', () => {
  for (const mode of ['light', 'dark'] as const) {
    it.each(CASES)(`has no violations: %s (${mode})`, async (_name, show) => {
      document.documentElement.dataset.mode = mode;
      prefs.set({ mode });
      const view = await show();
      expect(await violations(view)).toEqual([]);
    });
  }
});
