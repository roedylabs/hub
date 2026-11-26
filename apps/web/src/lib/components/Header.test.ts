// Taken over from Roedy Mail (`apps/web/src/lib/components/Header.test.ts`), trimmed to the hub's header.
import { fireEvent, render } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import Header from '#lib/components/Header.svelte';
import type { HeaderValues } from '#lib/shell.ts';

function values(patch: Partial<HeaderValues> = {}): HeaderValues {
  return {
    isMobile: false,
    appName: 'Example Org',
    logo: { isMark: true },
    accountOpen: false,
    userName: 'Doe, Jane',
    initials: 'DJ',
    showAdmin: true,
    adminOn: false,
    settingsOn: false,
    ...patch,
  };
}

describe('header', () => {
  it('shows brand, toolbar and avatar on the desktop', async () => {
    const goHome = vi.fn();
    const openAdmin = vi.fn();
    const openSettingsBtn = vi.fn();
    const { getByRole, getByText } = render(Header, { ...values(), goHome, openAdmin, openSettingsBtn });
    await fireEvent.click(getByRole('button', { name: 'Example Org' }));
    expect(goHome).toHaveBeenCalled();
    await fireEvent.click(getByRole('button', { name: 'Administration' }));
    expect(openAdmin).toHaveBeenCalled();
    await fireEvent.click(getByRole('button', { name: 'Settings' }));
    expect(openSettingsBtn).toHaveBeenCalled();
    // Initials as decoration, the name only for screen readers (WCAG 2.5.3).
    const avatar = getByRole('button', { name: 'Doe, Jane' });
    expect(avatar).not.toHaveAttribute('aria-label');
    expect(getByText('DJ')).toHaveAttribute('aria-hidden', 'true');
    // No search, notifications or apps in the hub.
    expect(document.querySelector('[role="combobox"]')).toBeNull();
  });

  it('connects the avatar with its popup (disclosure pattern)', async () => {
    const accountPop = createRawSnippet((id: () => string) => ({
      render: () => `<div id="${id()}"><button>Manage account</button></div>`,
    }));
    const toggleAccount = vi.fn();
    const { getByRole, queryByRole, rerender } = render(Header, { ...values(), accountPop, toggleAccount });
    const avatar = getByRole('button', { name: 'Doe, Jane' });
    expect(avatar).toHaveAttribute('aria-expanded', 'false');
    expect(avatar).not.toHaveAttribute('aria-controls');
    expect(queryByRole('button', { name: 'Manage account' })).toBeNull();
    await fireEvent.click(avatar);
    expect(toggleAccount).toHaveBeenCalled();
    await rerender({ ...values({ accountOpen: true }), accountPop, toggleAccount });
    expect(avatar).toHaveAttribute('aria-expanded', 'true');
    expect(avatar).toHaveClass('ring');
    const manage = getByRole('button', { name: 'Manage account' });
    expect(document.getElementById(avatar.getAttribute('aria-controls') ?? '')).toContainElement(manage);
  });

  it('puts focus back on its toggle with Escape in the popup', async () => {
    const accountPop = createRawSnippet(() => ({ render: () => '<div><button>Manage account</button></div>' }));
    const onWindow = vi.fn();
    window.addEventListener('keydown', onWindow);
    try {
      const { getByRole } = render(Header, { ...values({ accountOpen: true }), accountPop });
      const manage = getByRole('button', { name: 'Manage account' });
      manage.focus();
      await fireEvent.keyDown(manage, { key: 'Tab' });
      expect(manage).toHaveFocus();
      await fireEvent.keyDown(manage, { key: 'Escape' });
      expect(getByRole('button', { name: 'Doe, Jane' })).toHaveFocus();
      // Closing stays with the layout: the key goes on.
      expect(onWindow).toHaveBeenCalledTimes(2);
    } finally {
      window.removeEventListener('keydown', onWindow);
    }
  });

  it('hides the administration for users without administrator rights', () => {
    const { queryByRole } = render(Header, values({ showAdmin: false }));
    expect(queryByRole('button', { name: 'Administration' })).toBeNull();
  });

  it('marks the open area', async () => {
    const { getByRole, rerender } = render(Header, values({ settingsOn: true }));
    expect(getByRole('button', { name: 'Settings' })).toHaveAttribute('aria-current', 'page');
    expect(getByRole('button', { name: 'Administration' })).not.toHaveAttribute('aria-current');
    await rerender(values({ adminOn: true }));
    expect(getByRole('button', { name: 'Administration' })).toHaveAttribute('aria-current', 'page');
    expect(getByRole('button', { name: 'Settings' })).not.toHaveAttribute('aria-current');
  });

  it('shows only the brand and the avatar on mobile, which opens the account sheet', async () => {
    const openAccountSheet = vi.fn();
    const goHome = vi.fn();
    const { getByRole, queryByRole } = render(Header, { ...values({ isMobile: true }), openAccountSheet, goHome });
    expect(queryByRole('button', { name: 'Administration' })).toBeNull();
    expect(queryByRole('button', { name: 'Settings' })).toBeNull();
    await fireEvent.click(getByRole('button', { name: 'Example Org' }));
    expect(goHome).toHaveBeenCalled();
    const avatar = getByRole('button', { name: 'Doe, Jane' });
    expect(avatar).toHaveAttribute('aria-haspopup', 'dialog');
    await fireEvent.click(avatar);
    expect(openAccountSheet).toHaveBeenCalled();
  });
});
