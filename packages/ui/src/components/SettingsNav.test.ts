// Taken over from Roedy Mail (`apps/web/src/lib/components/SettingsNav.test.ts`), with test groups instead of an
// app's pages.
import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import SettingsNav from './SettingsNav.svelte';
import { BRAND0 } from '../branding.ts';
import { mkLogo } from '../logo.ts';
import type { NavEntry, NavGroupValues, SettingsNavValues } from '../shell.ts';

/** Groups like an app's: `current` is the open page, `open` gets the page's id. */
function groups(current: string, open: (id: string) => void = () => {}): NavGroupValues[] {
  const entry = (id: string, label: string): NavEntry => ({
    id,
    label,
    icon: 'ti-user',
    on: id === current,
    fw: id === current ? 800 : 600,
    onClick: () => open(id),
  });
  return [
    { label: 'Account', hasLabel: true, items: [entry('profile', 'Profile'), entry('security', 'Security')] },
    { label: '', hasLabel: false, items: [entry('users', 'Users')] },
    { label: 'System', hasLabel: true, items: [entry('email', 'Email')] },
  ];
}

function values(patch: Partial<SettingsNavValues> = {}): SettingsNavValues {
  return {
    isMobile: false,
    appName: 'Example Org',
    logo: mkLogo(BRAND0, 'Example Org'),
    backLabel: 'Back to home',
    panelBack: () => {},
    mPanelH: 'Administration',
    settingsGroups: groups('users'),
    ...patch,
  };
}

describe('settings navigation component', () => {
  it('shows "Back to home", the groups and the app on the desktop', async () => {
    const open = vi.fn();
    const panelBack = vi.fn();
    const view = render(SettingsNav, values({ panelBack, settingsGroups: groups('users', open) }));
    expect(view.getByRole('navigation', { name: 'Administration' })).toBeInTheDocument();
    expect(view.getByRole('button', { name: 'Users' })).toHaveAttribute('aria-current', 'page');
    expect(view.getByRole('button', { name: 'Email' })).not.toHaveAttribute('aria-current');
    expect(view.getByText('System')).toBeInTheDocument();
    expect(view.getByText('Example Org')).toBeInTheDocument();
    expect(view.queryByRole('heading')).toBeNull();
    await fireEvent.click(view.getByRole('button', { name: 'Email' }));
    expect(open).toHaveBeenCalledWith('email');
    await fireEvent.click(view.getByRole('button', { name: 'Back to home' }));
    expect(panelBack).toHaveBeenCalled();
  });

  it('shows the area’s list with its name as the heading on mobile', async () => {
    const open = vi.fn();
    const view = render(SettingsNav, values({ isMobile: true, mPanelH: 'Settings', settingsGroups: groups('', open) }));
    expect(view.getByRole('heading', { level: 1, name: 'Settings' })).toBeInTheDocument();
    expect(view.queryByRole('navigation')).toBeNull();
    expect(view.queryByRole('button', { name: 'Back to home' })).toBeNull();
    expect(view.getAllByRole('button').map((b) => b.textContent?.trim())).toEqual([
      'Profile',
      'Security',
      'Users',
      'Email',
    ]);
    expect(view.getByText('Account')).toBeInTheDocument();
    await fireEvent.click(view.getByRole('button', { name: 'Security' }));
    expect(open).toHaveBeenCalledWith('security');
  });
});
