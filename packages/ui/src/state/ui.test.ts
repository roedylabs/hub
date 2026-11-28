// Taken over from Roedy Mail (`apps/web/src/lib/state/ui.test.ts`), trimmed to menus, confirmation and loading.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Loading, SKELETON_DELAY, Ui } from './ui.svelte.ts';

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('ui', () => {
  it('opens and closes menus', () => {
    const ui = new Ui();
    ui.toggleMenu('account');
    expect(ui.menu).toBe('account');
    ui.toggleMenu('pick');
    expect(ui.menu).toBe('pick');
    ui.toggleMenu('pick');
    expect(ui.menu).toBeNull();
    ui.menu = 'account';
    ui.closeMenu();
    expect(ui.menu).toBeNull();
  });
});

describe('ui: confirmation', () => {
  it('asks and runs only after "Confirm", closing an open menu', () => {
    const ui = new Ui();
    const run = vi.fn();
    ui.menu = 'account';
    ui.ask({ title: 'Sign out device?', text: '', run });
    expect(ui.confirm).not.toBeNull();
    expect(ui.menu).toBeNull();
    ui.confirmCancel();
    expect(ui.confirm).toBeNull();
    ui.ask({ title: 'Delete account?', text: '', word: 'DELETE', run });
    ui.confirmOk();
    expect(run).not.toHaveBeenCalled();
    ui.confirmIn = ' delete ';
    expect(ui.confirmReady).toBe(true);
    ui.confirmOk();
    expect(run).toHaveBeenCalledOnce();
    expect(ui.confirm).toBeNull();
    expect(ui.confirmIn).toBe('');
  });

  it('always asks: the hub has no "Ask before deleting" setting', () => {
    const ui = new Ui();
    const run = vi.fn();
    ui.ask({ title: 'Remove user?', text: '', run });
    expect(run).not.toHaveBeenCalled();
    expect(ui.confirm?.title).toBe('Remove user?');
  });

  it('compares the word case-insensitively', () => {
    const ui = new Ui();
    ui.ask({ title: 'Delete?', text: '', word: 'Straße', run: () => {} });
    ui.confirmIn = 'STRASSE';
    expect(ui.confirmReady).toBe(true);
  });

  it('cannot be closed without cancelling', () => {
    const ui = new Ui();
    ui.ask({ title: 'Session expired', text: '', noCancel: true, run: () => {} });
    ui.confirmCancel();
    expect(ui.confirm).not.toBeNull();
  });

  it('discards menus, dialogs, confirmation and toast on sign-out', () => {
    const ui = new Ui();
    ui.menu = 'account';
    ui.pick = () => ({ title: 'Role', opts: [] });
    ui.pageDialog = () => null;
    ui.ask({ title: 'T', text: '', run: () => {} });
    ui.confirmIn = 'abc';
    ui.toasts.show('Notice');
    ui.reset();
    expect(ui).toMatchObject({ menu: null, pick: null, pageDialog: null, confirm: null, confirmIn: '', toast: null });
  });
});

describe('Loading', () => {
  it('shows the skeleton only after 150 ms', async () => {
    const loading = new Loading();
    let done!: (v: string) => void;
    const result = loading.run(new Promise<string>((resolve) => (done = resolve)));
    vi.advanceTimersByTime(SKELETON_DELAY - 1);
    expect(loading.slow).toBe(false);
    vi.advanceTimersByTime(1);
    expect(loading.slow).toBe(true);
    done('ok');
    await expect(result).resolves.toBe('ok');
    expect(loading.slow).toBe(false);
  });

  it('shows no skeleton for fast responses', async () => {
    const loading = new Loading();
    await loading.run(Promise.resolve(1));
    vi.advanceTimersByTime(SKELETON_DELAY);
    expect(loading.slow).toBe(false);
  });
});
