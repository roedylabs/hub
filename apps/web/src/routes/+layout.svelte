<!--
  Root layout of the hub's web app: loads the public configuration and the
  session, applies theme and language, and redirects – without an
  administrator to the setup, without sign-in from the pages that need a user
  to the sign-in (with `next`), signed in from the sign-in to `next`
  (`#lib/routes.ts`). Until the session is there the page stays empty; if it
  cannot be loaded, the error page shows ("offline" or 500) with "Try again".
  Toasts and tooltips sit above every page.

  Taken over from Roedy Mail (`apps/web/src/routes/+layout.svelte`) without
  the shell (header, sidebar, menus, sheets, mail accounts, push).
-->
<script lang="ts">
  import { untrack, type Snippet } from 'svelte';
  import '@fontsource-variable/nunito-sans/opsz.css';
  import '@fontsource/young-serif/400.css';
  import '../app.css';
  import { afterNavigate, goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import type { TipState } from '#lib/actions/tooltip.ts';
  import ErrorView from '#lib/components/ErrorView.svelte';
  import Toast from '#lib/components/Toast.svelte';
  import Tooltip from '#lib/components/Tooltip.svelte';
  import { failText, gatewayErrorIn } from '#lib/errors.ts';
  import { leaveTo } from '#lib/navigate.ts';
  import { isReauth, loginHref, nextOf } from '#lib/next.ts';
  import { m } from '#lib/paraglide/messages.js';
  import { pageTitle, redirectFor } from '#lib/routes.ts';
  // Registers what happens when the session ends in another tab or from outside.
  import '#lib/state/account.ts';
  import { prefs } from '#lib/state/prefs.svelte.ts';
  import { session } from '#lib/state/session.svelte.ts';
  import { ui } from '#lib/state/ui.svelte.ts';
  import { applyTheme, themeOf } from '#lib/theme.ts';

  let { children }: { children: Snippet } = $props();

  // An error the gateway answered with: the page shows it without loading anything else.
  const gatewayError = gatewayErrorIn();

  $effect(() => {
    if (gatewayError) return;
    untrack(() => {
      void session.load().then(() => {
        // The account's settings (theme, language); without them the defaults apply.
        if (session.authed) prefs.load().catch(() => {});
      });
    });
    return session.watchTabs();
  });

  afterNavigate(({ from }) => {
    if (from) ui.navigated = true;
  });

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

  const title = $derived(pageTitle({ path, appName: session.appName, auth: ui.authTitle }));
  /** The page shows while nothing redirects; the error pages always. */
  const showPage = $derived(!!gatewayError || !!page.error || (ready && !target));
</script>

<svelte:head>
  {#if !gatewayError && !page.error}<title>{title}</title>{/if}
</svelte:head>

{#if session.status === 'error' && !gatewayError && !page.error}
  <ErrorView code={session.failure ?? '500'} retry={() => session.load()} />
{:else if showPage}
  <div class="page">{@render children()}</div>
{/if}

<div class="layer">
  {#if ui.toast}
    {#key ui.toast.id}
      <Toast toast={ui.toast} isMobile={ui.isMobile} />
    {/key}
  {/if}
</div>

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

  /* Toasts above pages that scroll themselves */
  .layer {
    position: fixed;
    inset: var(--sat) var(--sar) var(--sab) var(--sal);
    z-index: 95;
    pointer-events: none;
  }

  .layer > :global(*) {
    pointer-events: auto;
  }
</style>
