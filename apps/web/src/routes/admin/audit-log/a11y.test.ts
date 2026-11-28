/**
 * axe-core over Administration › Audit log in each state (entries with a
 * further page and a search, first load, empty, list and retention failed,
 * loading more), desktop and mobile, in the light and the dark theme. Colour
 * contrast is left to the browser audit (`scripts/a11y/scenarios/audit.mjs`).
 */
import { fireEvent, type RenderResult } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AuditEntry } from '#lib/audit.svelte.ts';
import { prefs } from '#lib/state/prefs.svelte.ts';
import { violations } from '@roedy/ui/testing';
import { renderSignedIn, resetSignedIn, setPhone, type ApiRoutes } from '#lib/test/signed-in.ts';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(() => Promise.resolve()) }));

afterEach(() => {
  delete document.documentElement.dataset.mode;
  resetSignedIn();
});

const at = (hoursAgo: number) => new Date(Date.now() - hoursAgo * 3_600_000).toISOString();
const ENTRIES: AuditEntry[] = [
  {
    id: 3,
    at: at(1),
    cat: 'login',
    icon: 'ti-login',
    who: 'Jordan Hale',
    act: ' signed in',
    meta: '192.0.2.x',
    warn: false,
  },
  {
    id: 2,
    at: at(2),
    cat: 'users',
    icon: 'ti-user-shield',
    who: 'System',
    act: ': administrator access reset',
    meta: '',
    warn: true,
  },
  {
    id: 1,
    at: at(30),
    cat: 'apps',
    icon: 'ti-apps',
    who: 'Alex Example',
    act: ' opened an app',
    meta: '',
    warn: false,
  },
];
const COUNTS = { all: 3, login: 1, users: 1, apps: 1, settings: 0, system: 0 };
const LIST = { body: { entries: ENTRIES, counts: COUNTS, next: 1 } };
/** Hits of the search: still a further page. */
const FOUND = { body: { entries: ENTRIES.slice(0, 1), counts: { ...COUNTS, all: 1, users: 0, apps: 0 }, next: 3 } };
const KEEP = { body: { days: 90 } };
const never = () => new Promise<never>(() => {});

type Show = () => Promise<RenderResult<never>>;

/** `routes` as a function: `mockApi` uses up its queues, every run needs its own. */
function show(routes: () => ApiRoutes, ready: (view: RenderResult<never>) => Promise<unknown>): Show {
  return async () => {
    const view = renderSignedIn(Page, { path: '/admin/audit-log', api: routes() }) as unknown as RenderResult<never>;
    await ready(view);
    return view;
  };
}

const CASES: [string, Show][] = [
  [
    'entries with a search',
    show(
      () => ({ 'GET admin/audit': [LIST], 'GET admin/audit?q=jordan': [FOUND], 'GET admin/audit/retention': [KEEP] }),
      async (view) => {
        await view.findByText('Jordan Hale');
        await view.findByText('Entries older than 90 days are deleted');
        await fireEvent.input(view.getByRole('textbox'), { target: { value: 'jordan' } });
        // The scan runs on the hits, not while the search is still on its way.
        await vi.waitFor(() => expect(view.queryByText('Alex Example')).toBeNull());
        expect(view.getByText('Jordan Hale')).toBeInTheDocument();
      },
    ),
  ],
  [
    'first load',
    show(
      () => ({ 'GET admin/audit': [never], 'GET admin/audit/retention': [never] }),
      async () => {},
    ),
  ],
  [
    'empty',
    show(
      () => ({
        'GET admin/audit': [{ body: { entries: [], counts: { ...COUNTS, all: 0 }, next: null } }],
        'GET admin/audit/retention': [KEEP],
      }),
      (view) => view.findByText('No entries found'),
    ),
  ],
  [
    'failed',
    show(
      () => ({ 'GET admin/audit': [{ status: 500 }], 'GET admin/audit/retention': [{ status: 500 }] }),
      async (view) => {
        await vi.waitFor(() => expect(view.getAllByRole('alert')).toHaveLength(2));
      },
    ),
  ],
  [
    'loading more',
    show(
      () => ({ 'GET admin/audit': [LIST], 'GET admin/audit?before=1': [never], 'GET admin/audit/retention': [KEEP] }),
      async (view) => {
        await fireEvent.click(await view.findByRole('button', { name: 'Load more entries' }));
        await view.findByText('Loading …');
      },
    ),
  ],
];

describe('Audit log page accessibility', () => {
  for (const mobile of [false, true]) {
    for (const mode of ['light', 'dark'] as const) {
      for (const [name, render] of CASES) {
        it(`${name} (${mobile ? 'mobile' : 'desktop'}, ${mode})`, async () => {
          setPhone(mobile);
          document.documentElement.dataset.mode = mode;
          prefs.set({ mode });
          const view = await render();
          expect(await violations(view)).toEqual([]);
        });
      }
    }
  }
});
