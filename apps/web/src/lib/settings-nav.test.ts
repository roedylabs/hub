import { describe, expect, it, vi } from 'vitest';
import {
  FIRST_PAGE,
  isSectionHome,
  navGroupValues,
  pageOf,
  panelGroups,
  sectionName,
  sectionOf,
} from '#lib/settings-nav.ts';

describe('settings navigation', () => {
  it('structures settings and administration into groups and pages', () => {
    expect(panelGroups('account').map((g) => [g.label, g.pages.map((p) => [p.label, p.route, p.icon])])).toEqual([
      [
        'Account',
        [
          ['Profile', '/account/profile', 'ti-user'],
          ['Security', '/account/security', 'ti-shield'],
          ['Accounts', '/account/accounts', 'ti-at'],
        ],
      ],
      [
        'Display & language',
        [
          ['Appearance', '/account/appearance', 'ti-palette'],
          ['Language & region', '/account/language', 'ti-language'],
          ['General', '/account/general', 'ti-adjustments-horizontal'],
        ],
      ],
    ]);
    expect(panelGroups('admin').map((g) => [g.label, g.pages.map((p) => p.id)])).toEqual([
      [null, ['users', 'apps', 'organization', 'branding']],
      ['System', ['stalwart', 'email', 'audit-log']],
    ]);
    expect(panelGroups('admin').flatMap((g) => g.pages.map((p) => p.icon))).toEqual([
      'ti-users',
      'ti-apps',
      'ti-building',
      'ti-brush',
      'ti-server-2',
      'ti-mail',
      'ti-list-details',
    ]);
  });

  it('gives every page a subtitle', () => {
    for (const section of ['account', 'admin'] as const) {
      for (const p of panelGroups(section).flatMap((g) => g.pages)) expect(p.desc, p.id).not.toBe('');
    }
  });

  it('finds the area and the page of an address, also of subpages', () => {
    expect(sectionOf('/account')).toBe('account');
    expect(sectionOf('/account/security')).toBe('account');
    expect(sectionOf('/admin')).toBe('admin');
    expect(sectionOf('/admin/audit-log')).toBe('admin');
    // The guest page "Confirm email" has no shell.
    expect(sectionOf('/account/confirm-email/tok')).toBeNull();
    // "No account yet" neither; the accounts' subpages belong to Account › Accounts.
    expect(sectionOf('/account/no-account')).toBeNull();
    expect(pageOf('/account/accounts/new')).toMatchObject({ id: 'accounts' });
    expect(pageOf('/account/accounts/01JACCT')).toMatchObject({ id: 'accounts' });
    expect(sectionOf('/')).toBeNull();
    expect(sectionOf('/accounts')).toBeNull();
    expect(sectionOf('/administration')).toBeNull();
    expect(pageOf('/account/profile')).toMatchObject({ id: 'profile', label: 'Profile' });
    expect(pageOf('/admin/users/01J')?.id).toBe('users');
    expect(pageOf('/account')).toBeNull();
    expect(pageOf('/account/confirm-email/tok')).toBeNull();
    expect(pageOf('/login')).toBeNull();
  });

  it('knows the start of each area and its first page', () => {
    expect(isSectionHome('/account')).toBe(true);
    expect(isSectionHome('/admin/')).toBe(true);
    expect(isSectionHome('/account/profile')).toBe(false);
    expect(isSectionHome('/')).toBe(false);
    expect(FIRST_PAGE).toEqual({ account: '/account/profile', admin: '/admin/users' });
    expect(sectionName('account')).toBe('Settings');
    expect(sectionName('admin')).toBe('Administration');
  });

  it('marks the current page and opens the others', () => {
    const open = vi.fn();
    const groups = navGroupValues({ path: '/account/appearance', open });
    const items = groups.flatMap((g) => g.items);
    expect(items.filter((i) => i.on)).toEqual([expect.objectContaining({ id: 'appearance', fw: 800 })]);
    expect(items.filter((i) => !i.on).every((i) => i.fw === 600)).toBe(true);
    expect(groups.map((g) => [g.label, g.hasLabel])).toEqual([
      ['Account', true],
      ['Display & language', true],
    ]);
    items.find((i) => i.id === 'security')!.onClick();
    expect(open).toHaveBeenCalledWith(expect.objectContaining({ route: '/account/security' }));
    // The list of an area marks nothing; outside the areas there are no groups.
    expect(
      navGroupValues({ path: '/admin', open })
        .flatMap((g) => g.items)
        .some((i) => i.on),
    ).toBe(false);
    expect(navGroupValues({ path: '/admin', open })[0]).toMatchObject({ label: '', hasLabel: false });
    expect(navGroupValues({ path: '/', open })).toEqual([]);
  });
});
