<!--
  Root layout of the hub's web app: loads the public configuration and the
  session, applies theme and language, and redirects – without an
  administrator to the setup, without sign-in from the pages that need a user
  to the sign-in (with `next`), signed in from the sign-in to `next`
  (`#lib/routes.ts`). Until the session is there the page stays empty; if it
  cannot be loaded, the error page shows ("offline" or 500) with "Try again".

  Settings (`/account/…`) and administration (`/admin/…`) get the shell: on
  the desktop the header, the area's navigation and the content card; on
  mobile a "Back" header and a panel with the area's list (on `/account` and
  `/admin`) or the page. On the desktop `/account` and `/admin` lead on to
  their first page. A user who is no administrator sees "Access denied" in
  place of an administration page. Exactly one `main` per route: the shell's,
  otherwise the page's own (`#lib/landmark.ts`).

  Toasts, the confirmation (`ui.ask`), a page's dialog (`ui.pageDialog`) and
  select list (`ui.pick`, mobile), the account menu and tooltips sit above
  every page. A click outside a popover and Escape close it.

  Taken over from Roedy Mail (`apps/web/src/routes/+layout.svelte`) without
  the mailbox (sidebar, tab bar, search, notifications, apps, mail accounts),
  the splash screen, push, the command palette and keyboard shortcuts, and
  the swipe gestures of the shell.
-->
<script lang="ts">
  import { untrack, type Snippet } from 'svelte';
  import '@fontsource-variable/nunito-sans/opsz.css';
  import '@fontsource/young-serif/400.css';
  import '../app.css';
  import { afterNavigate, goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import { scrollbar } from '#lib/actions/scrollbar.ts';
  import type { TipState } from '#lib/actions/tooltip.ts';
  import AccountMenu from '#lib/components/AccountMenu.svelte';
  import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
  import ErrorView from '#lib/components/ErrorView.svelte';
  import Header from '#lib/components/Header.svelte';
  import Icon from '#lib/components/Icon.svelte';
  import PickerList from '#lib/components/PickerList.svelte';
  import SettingsNav from '#lib/components/SettingsNav.svelte';
  import Sheet from '#lib/components/Sheet.svelte';
  import Toast from '#lib/components/Toast.svelte';
  import Tooltip from '#lib/components/Tooltip.svelte';
  import { failText, gatewayErrorIn } from '#lib/errors.ts';
  import { setInMain } from '#lib/landmark.ts';
  import { langChoices } from '#lib/language.ts';
  import { logout } from '#lib/logout.ts';
  import { leaveTo } from '#lib/navigate.ts';
  import { isReauth, loginHref, nextOf } from '#lib/next.ts';
  import { m } from '#lib/paraglide/messages.js';
  import { pageTitle, redirectFor } from '#lib/routes.ts';
  import {
    FIRST_PAGE,
    SECTION_HOME,
    isSectionHome,
    navGroupValues,
    sectionName,
    sectionOf,
    type PanelPage,
    type SettingsNavValues,
  } from '#lib/settings-nav.ts';
  import { confirmValues, overlayClose, type AccountMenuValues, type HeaderValues } from '#lib/shell.ts';
  // Registers what happens when the session ends in another tab or from outside.
  import '#lib/state/account.ts';
  import { prefs } from '#lib/state/prefs.svelte.ts';
  import { session } from '#lib/state/session.svelte.ts';
  import { ui } from '#lib/state/ui.svelte.ts';
  import { applyTheme, themeOf } from '#lib/theme.ts';
  import { toastLift } from '#lib/toast-lift.svelte.ts';

  let { children }: { children: Snippet } = $props();

  // An error the gateway answered with: the page shows it without loading anything else.
  const gatewayError = gatewayErrorIn();

  /** Session, then the account's settings (theme, language); without them the defaults apply. */
  async function start(): Promise<void> {
    await session.load();
    if (session.authed) await prefs.load().catch(() => {});
  }

  $effect(() => {
    if (gatewayError) return;
    untrack(() => void start());
    return session.watchTabs();
  });

  /**
   * Path of the page before the current history entry as far as the app saw it (`null` at the start or unknown):
   * after browser Back or a jump of several entries the page left is not the one before.
   */
  let cameFrom: string | null = null;

  // In the SPA `from.url` is `null` at the start, unlike in SvelteKit's type.
  afterNavigate((navigation) => {
    const { from } = navigation;
    if (from) ui.navigated = true;
    const unknown = navigation.type === 'popstate' && navigation.delta !== 1;
    cameFrom = unknown ? null : (from?.url?.pathname ?? null);
  });

  /** Navigation API (where the browser has it): the history entries of the app and the current one. */
  interface NavigationLike {
    currentEntry?: { index: number } | null;
    entries?: () => { url: string | null }[];
  }

  /** Path of the history entry before the current one, `null` if unknown or not in the app. */
  function predecessor(): string | null {
    const nav = (globalThis as { navigation?: NavigationLike }).navigation;
    if (!nav?.currentEntry || typeof nav.entries !== 'function') return cameFrom;
    const url = nav.entries()[nav.currentEntry.index - 1]?.url;
    if (!url) return null;
    const prev = new URL(url);
    return prev.origin === location.origin ? prev.pathname : null;
  }

  const path = $derived(page.url.pathname);
  const ready = $derived(session.status === 'ready');
  const target = $derived(
    ready && !gatewayError && !page.error
      ? redirectFor(path, {
          setupRequired: session.setupRequired,
          authed: session.authed,
          reauth: isReauth(page.url),
          leavingSetup: ui.setupLeaving,
          here: path + page.url.search + page.url.hash,
        })
      : null,
  );

  $effect(() => {
    if (!target) return;
    untrack(() => {
      switch (target.to) {
        case 'setup':
          void goto(resolve('/setup'), { replace: true });
          break;
        case 'home':
          void goto(resolve('/'), { replace: true });
          break;
        case 'login':
          // eslint-disable-next-line svelte/no-navigation-without-resolve -- `/login` with the checked `next`
          void goto(loginHref(target.next), { replace: true });
          break;
        case 'next':
          leaveTo(nextOf(page.url));
          break;
      }
    });
  });

  $effect(() => {
    applyTheme(document.documentElement, themeOf(prefs.values, session.branding, ui.systemDark.current));
  });

  // Language on `<html lang>` (WCAG 3.1.1).
  $effect(() => {
    document.documentElement.lang = prefs.lang;
  });

  // Report failed saving of the settings; the fields are already reset.
  $effect(() => {
    if (prefs.error) untrack(() => ui.toasts.error(failText(prefs.error, m.error_pref())));
  });

  /** The page shows while nothing redirects; the error pages always. */
  const showPage = $derived(!!gatewayError || !!page.error || (ready && !target));

  // ---- Shell of settings and administration ----

  /** Area of the address (`/account/…`, `/admin/…`); `null` for the other pages and the guest pages. */
  const section = $derived(sectionOf(path));
  /** The shell: a signed-in account on a page of the area, no error page of the router or the gateway. */
  const shell = $derived(!gatewayError && !page.error && ready && !target && session.authed && section !== null);
  /** An administration page for an account without administrator rights: "Access denied" (the server says 403 too). */
  const forbidden = $derived(shell && section === 'admin' && !session.isAdmin);
  /** Mobile: the area's list on its start (`/account`, `/admin`) instead of a page. */
  const panelList = $derived(shell && !forbidden && ui.isMobile && isSectionHome(path));
  // Exactly one `main` per route: the shell's around the page (`#lib/landmark.ts`).
  setInMain(() => shell);

  // On the desktop the start of an area leads on to its first page, in place of its address.
  $effect(() => {
    if (shell && !forbidden && section && !ui.isMobile && isSectionHome(path)) {
      const to = FIRST_PAGE[section];
      untrack(() => void goto(resolve(to), { replace: true }));
    }
  });

  // Another page closes the open menu (account menu, select list).
  $effect(() => {
    void path;
    untrack(() => ui.closeMenu());
  });

  /** Opens `to` (resolved with `resolve()`) and closes the open menu; the open address itself replaces its entry. */
  function go(to: string): void {
    ui.closeMenu();
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- callers pass resolve() results
    void (to === path ? goto(to, { replace: true }) : goto(to));
  }

  /** Back to `to`: through the history when the entry before is `to`, so that it does not grow with every "Back". */
  function back(to: string): void {
    if (predecessor() === to && ui.canGoBack()) {
      ui.closeMenu();
      history.back();
    } else go(to);
  }

  const HOME = resolve('/');
  const openPage = (p: PanelPage) => go(resolve(p.route));
  /** Mobile "Back": from a page to its area's list, from the list (and "Access denied") home. */
  function panelBack() {
    if (section && !panelList && !forbidden) back(resolve(SECTION_HOME[section]));
    else back(HOME);
  }
  const mBackLabel = $derived(section && !panelList && !forbidden ? sectionName(section) : m.shell_back_home());

  const settingsNav: SettingsNavValues = $derived({
    isMobile: ui.isMobile,
    appName: session.appName,
    logo: session.logo,
    backLabel: m.shell_back_home(),
    panelBack: () => back(HOME),
    mPanelH: sectionName(section ?? 'account'),
    settingsGroups: navGroupValues({ path, open: openPage }),
  });

  const header: HeaderValues = $derived({
    isMobile: ui.isMobile,
    isTablet: ui.isTablet,
    appName: session.appName,
    logo: session.logo,
    accountOpen: ui.menu === 'account',
    userName: session.userName,
    initials: session.initials,
    showAdmin: session.isAdmin,
    adminOn: section === 'admin',
    settingsOn: section === 'account',
  });

  const account: AccountMenuValues = $derived({
    isMobile: ui.isMobile,
    initials: session.initials,
    userName: session.userName,
    userEmail: session.user?.email ?? '',
    modeSeg: (
      [
        ['light', 'ti-sun', m.account_light(), !ui.dark],
        ['dark', 'ti-moon', m.account_dark(), ui.dark],
      ] as const
    ).map(([mode, icon, label, on]) => ({
      label,
      icon,
      on,
      onClick: () => {
        if (prefs.values.mode !== mode) prefs.set({ mode });
      },
    })),
    langSeg: langChoices(prefs.values.lang, prefs.browserLang, (patch) => prefs.set(patch)),
    openAccountSettings: () => go(resolve('/account/profile')),
    openAdmin: ui.isMobile && session.isAdmin ? () => go(resolve(FIRST_PAGE.admin)) : undefined,
    logout: () => void logout(),
  });

  /** Open dialog of the page (`ui.pageDialog`). */
  const pageDialog = $derived(ui.pageDialog?.() ?? null);
  /** Mobile sheet of the shell: the account menu or a page's select list. */
  const sheet = $derived(ui.isMobile && shell ? ui.menu : null);

  /** A click outside a popover closes it (`[data-pop]` marks the popovers and their toggles). */
  function onDocDown(event: MouseEvent) {
    if (!ui.menu || ui.isMobile) return;
    if (event.target instanceof Element && event.target.closest('[data-pop]')) return;
    ui.closeMenu();
  }

  /**
   * Escape closes the topmost open thing: the confirmation, the page's dialog, the open menu. A popover has
   * already put the focus back on its toggle (`Header`), a sheet closes itself.
   */
  function onKey(event: KeyboardEvent) {
    if (event.key !== 'Escape') return;
    if (ui.confirm) {
      event.preventDefault();
      ui.confirmCancel();
    } else if (event.defaultPrevented) return;
    else if (pageDialog) pageDialog.close();
    else if (ui.menu) ui.closeMenu();
  }

  /** The content card (desktop) and the panel (mobile) start at the top on every other page or list. */
  let scroller = $state<HTMLElement | null>(null);
  $effect(() => {
    void path;
    void panelList;
    if (scroller) scroller.scrollTop = 0;
  });

  /** Document title per route (WCAG 2.4.2). */
  const title = $derived(
    pageTitle({ path, appName: session.appName, auth: ui.authTitle, error: forbidden ? '403' : null }),
  );
</script>

<svelte:head>
  {#if !gatewayError && !page.error}<title>{title}</title>{/if}
</svelte:head>

<svelte:document onmousedown={onDocDown} />
<svelte:window onkeydown={onKey} />

<!-- Page content; administration without admin rights "Access denied". -->
{#snippet content()}
  {#if forbidden}<ErrorView code="403" />{:else}{@render children()}{/if}
{/snippet}

{#snippet accountPop(id: string)}<AccountMenu {...account} {id} />{/snippet}

<!-- Toast and confirmation, in the shell and above the pages without it. -->
{#snippet overlays()}
  {#if ui.toast}
    {#key ui.toast.id}
      <Toast toast={ui.toast} isMobile={ui.isMobile} lift={toastLift.bottom} />
    {/key}
  {/if}
  {#if ui.confirm}
    <ConfirmDialog
      isMobile={ui.isMobile}
      cf={confirmValues(ui.confirm, ui.confirmReady, m.confirm_ok())}
      confirmIn={ui.confirmIn}
      onConfirmIn={(e: Event & { currentTarget: HTMLInputElement }) => (ui.confirmIn = e.currentTarget.value)}
      confirmKey={(e: KeyboardEvent) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          ui.confirmOk();
        }
      }}
      confirmOk={() => ui.confirmOk()}
      confirmCancel={() => ui.confirmCancel()}
      onConfirmOverlay={overlayClose(() => ui.confirmCancel())}
    />
  {/if}
{/snippet}

{#if session.status === 'error' && !gatewayError && !page.error}
  <ErrorView code={session.failure ?? '500'} retry={start} />
{:else if shell}
  <!-- One tree for desktop and mobile: the page in `main` stays mounted across the breakpoint (unsaved inputs stay). -->
  <div class="app" class:mobile={ui.isMobile}>
    {#if !ui.isMobile}
      <Header
        {...header}
        {accountPop}
        goHome={() => go(HOME)}
        openAdmin={() => go(resolve(FIRST_PAGE.admin))}
        openSettingsBtn={() => go(resolve(FIRST_PAGE.account))}
        toggleAccount={() => ui.toggleMenu('account')}
      />
    {/if}
    <div class={ui.isMobile ? 'm-page' : 'body'}>
      {#if !ui.isMobile}
        {#if !forbidden}<SettingsNav {...settingsNav} />{/if}
      {:else if panelList}
        <!-- The area's list: brand (home) and avatar (account menu with sign-out and, for administrators, administration). -->
        <Header {...header} goHome={() => go(HOME)} openAccountSheet={() => ui.toggleMenu('account')} />
      {:else}
        <header class="m-back">
          <button type="button" onclick={panelBack}>
            <span class="m-back-icon"><Icon name="ti-arrow-left" /></span>
            <span class="m-back-label">{mBackLabel}</span>
          </button>
        </header>
      {/if}
      <main
        class={['no-scrollbar', ui.isMobile ? 'm-panel' : 'card', !ui.isMobile && forbidden && 'wide']}
        bind:this={scroller}
        {@attach !ui.isMobile && scrollbar}
      >
        {#if panelList}<SettingsNav {...settingsNav} />{:else}<div
            class={['content', !ui.isMobile && !forbidden && 'panel']}
          >
            {@render content()}
          </div>{/if}
      </main>
    </div>
    {#if !ui.isMobile}
      {#if pageDialog}{@render pageDialog.content(false)}{/if}
    {:else}
      {#if sheet === 'account'}
        <Sheet label={session.userName} onclose={() => ui.closeMenu()}>
          <AccountMenu {...account} />
        </Sheet>
      {:else if sheet === 'pick' && ui.pick}
        {@const pick = ui.pick()}
        <Sheet label={pick.title} onclose={() => ui.closeMenu()}>
          <PickerList {pick} closeSheet={() => ui.closeMenu()} />
        </Sheet>
      {/if}
      {#if pageDialog?.bare}
        {@render pageDialog.content(true)}
      {:else if pageDialog}
        <Sheet label={pageDialog.title} onclose={pageDialog.close}>
          {@render pageDialog.content(true)}
        </Sheet>
      {/if}
    {/if}
    {@render overlays()}
  </div>
{:else if showPage}
  <div class="page">{@render children()}</div>
{/if}

{#if !shell}<div class="layer">{@render overlays()}</div>{/if}

{#if ui.tip}
  <Tooltip tip={ui.tip} onplace={(tip: TipState) => (ui.tip = tip)} />
{/if}

<style>
  /* Reserves the insets itself: a page taller than the window (its own minimum height) still scrolls clear of them */
  .page {
    min-height: 100dvh;
    padding: var(--sat) var(--sar) var(--sab) var(--sal);
    display: flex;
    flex-direction: column;
  }

  /* Toasts and the confirmation above pages that scroll themselves */
  .layer {
    position: fixed;
    inset: var(--sat) var(--sar) var(--sab) var(--sal);
    z-index: 95;
    pointer-events: none;
  }

  .layer > :global(*) {
    pointer-events: auto;
  }

  /* The shell fills the window inside its insets; the card and the panel scroll. */
  .app {
    position: relative;
    height: calc(100dvh - var(--sat) - var(--sab));
    margin: var(--sat) var(--sar) 0 var(--sal);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    font-size: 15px;
  }

  .body {
    position: relative;
    z-index: 1;
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .card {
    flex: 1;
    min-width: 0;
    position: relative;
    margin: 0 16px 16px 0;
    display: block;
    border-radius: 32px;
    background: var(--surface);
    box-shadow: var(--sh2);
    overflow: hidden auto;
  }

  /* "Access denied" without the navigation: the card takes the whole width */
  .card.wide {
    margin-left: 16px;
    display: flex;
    flex-direction: column;
  }

  /* Holds the page in every layout; without `.panel` (mobile, "Access denied") it takes no box of its own. */
  .content:not(.panel) {
    display: contents;
  }

  .panel {
    max-width: 780px;
    margin: 0 auto;
    padding: 44px 40px 96px;
  }

  .m-page {
    position: relative;
    z-index: 1;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .m-back {
    flex-shrink: 0;
    height: 60px;
    display: flex;
    align-items: center;
    padding: 0 12px;
  }

  .m-back button {
    height: 46px;
    max-width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 16px 0 5px;
    border-radius: 9999px;
    font-size: 15px;
    font-weight: 700;
    text-align: left;
    transition: background-color var(--dur, 220ms);
  }

  .m-back button:hover {
    background: var(--hov);
  }

  .m-back-icon {
    width: 36px;
    height: 36px;
    flex-shrink: 0;
    border-radius: 9999px;
    background: var(--surface);
    box-shadow: var(--sh1);
    display: grid;
    place-items: center;
    font-size: 18px;
  }

  .m-back-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .m-panel {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0 8px 8px;
    border-radius: 28px;
    background: var(--surface);
    box-shadow: var(--sh2);
    padding: 26px 18px 36px;
  }
</style>
