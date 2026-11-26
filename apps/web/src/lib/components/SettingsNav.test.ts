// Taken over from Roedy Mail (`apps/web/src/lib/components/SettingsNav.test.ts`), trimmed to the hub's navigation.
import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import SettingsNav from '#lib/components/SettingsNav.svelte';
import { mkLogo } from '#lib/logo.ts';
import { navGroupValues, sectionName, sectionOf, type SettingsNavValues } from '#lib/settings-nav.ts';
import { BRAND0 } from '#lib/state/session.svelte.ts';

function values(path: string, patch: Partial<SettingsNavValues> = {}): SettingsNavValues {
  return {
    isMobile: false,
    appName: 'Example Org',
    logo: mkLogo(BRAND0, 'Example Org'),
    backLabel: 'Back to home',
    panelBack: () => {},
    mPanelH: sectionName(sectionOf(path) ?? 'account'),
    settingsGroups: navGroupValues({ path, open: () => {} }),
    ...patch,
  };
}

describe('settings navigation component', () => {
  it('shows "Back to home", the groups and the hub on the desktop', async () => {
    const open = vi.fn();
    const panelBack = vi.fn();
    const view = render(SettingsNav, {
      ...values('/admin/users', { panelBack }),
      settingsGroups: navGroupValues({ path: '/admin/users', open }),
    });
    expect(view.getByRole('navigation', { name: 'Administration' })).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Users' })).toHaveAttribute('aria-current', 'page');
    expect(view.getByRole('button', { name: 'Organization' })).not.toHaveAttribute('aria-current');
    expect(view.getByText('System')).toBeInTheDocument();
    expect(view.getByText('Example Org')).toBeInTheDocument();
    expect(view.queryByRole('heading')).toBeNull();
    await fireEvent.click(view.getByRole('button', { name: 'Organization' }));
    expect(open).toHaveBeenCalledWith(expect.objectContaining({ route: '/admin/organization' }));
    await fireEvent.click(view.getByRole('button', { name: 'Back to home' }));
    expect(panelBack).toHaveBeenCalled();
  });

  it('shows the area’s list with its name as the heading on mobile', async () => {
    const open = vi.fn();
    const view = render(SettingsNav, {
      ...values('/account', { isMobile: true }),
      settingsGroups: navGroupValues({ path: '/account', open }),
    });
    expect(view.getByRole('heading', { level: 1, name: 'Settings' })).toBeInTheDocument();
    expect(view.queryByRole('navigation')).toBeNull();
    expect(view.queryByRole('button', { name: 'Back to home' })).toBeNull();
    expect(view.getAllByRole('button').map((b) => b.textContent?.trim())).toEqual([
      'Profile',
      'Security',
      'Appearance',
      'Language',
    ]);
    expect(view.getByText('Display & language')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Language' }));
    expect(open).toHaveBeenCalledWith(expect.objectContaining({ route: '/account/language' }));
  });
});
