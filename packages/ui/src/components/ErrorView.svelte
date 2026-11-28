<!--
  Error page: 404, 403, 500, the gateway's 502, 503 and 504, and offline from
  `ERRS` with kicker, tile, title and text. With "Try again" (500, 502, 503,
  504, offline: spins and shows "Loading", then `retry`, by default a reload)
  and "Go to overview", otherwise "Go to overview" and "Back". Plus "Contact
  support" when the organization provides an address. Desktop and mobile are
  one component (`ui.isMobile`).

  "Go to overview" leads to the home page, the layout sends guests from there
  to the sign-in; "Back" goes to the previous page, without one home.

  Taken over from Roedy Mail (`apps/web/src/lib/components/ErrorView.svelte`)
  without the developer log and "Copy details".
-->
<script lang="ts">
  import { scrollbar } from '../actions/scrollbar.ts';
  import Icon from './Icon.svelte';
  import { ERRS, type ErrorCode } from '../errors.ts';
  import { rootTag } from '../landmark.ts';
  import { texts } from '../i18n.ts';
  import { ui } from '../state/ui.svelte.ts';

  const t = texts({
    en: {
      back: 'Back',
      doc_title: ({ title, app }: { title: string; app: string }) => `${title} – ${app}`,
      home: 'Go to overview',
      mail_subject: ({ kicker, app }: { kicker: string; app: string }) => `${kicker} in ${app}`,
      retry: 'Try again',
      retrying: 'Loading',
      support: 'Contact support',
      support_question: 'Still having trouble?',
    },
    de: {
      back: 'Zurück',
      doc_title: ({ title, app }) => `${title} – ${app}`,
      home: 'Zur Übersicht',
      mail_subject: ({ kicker, app }) => `${kicker} in ${app}`,
      retry: 'Erneut versuchen',
      retrying: 'Wird geladen',
      support: 'Support kontaktieren',
      support_question: 'Besteht das Problem weiter?',
    },
  });

  interface Props {
    code: ErrorCode;
    /** Name of the app (or of the hub) in the document title and the support mail's subject. */
    appName: string;
    /** Address for "Contact support"; without one the link has no recipient. */
    support?: string | null;
    /** "Try again": runs until the promise is fulfilled; without it the page reloads. */
    retry?: () => unknown;
  }

  let { code, appName, support = null, retry }: Props = $props();

  // Main landmark when the layout does not put one around the page.
  const tag = rootTag();

  /** Minimum duration of "Loading" (at most 500 ms without animations). */
  const RETRY_MS = 900;

  let busy = $state(false);
  let rot = $state(0);

  const page = $derived(ERRS[code]);
  const kicker = $derived(page.kicker());
  const mail = $derived(
    `mailto:${support ?? ''}?subject=${encodeURIComponent(t.mail_subject({ kicker, app: appName }))}`,
  );

  /** Full page load: the home page may be another app's error page's address too. */
  function home() {
    location.assign('/');
  }

  /** To the previous page; without one (link opened directly) to the overview instead of leaving the hub. */
  function back() {
    if (ui.canGoBack()) history.back();
    else home();
  }

  async function run() {
    if (busy) return;
    busy = true;
    rot += 720;
    await new Promise((done) => setTimeout(done, ui.motion ? RETRY_MS : Math.min(RETRY_MS, 500)));
    try {
      await (retry ? retry() : location.reload());
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head>
  <title>{t.doc_title({ title: page.title(), app: appName })}</title>
</svelte:head>

<svelte:element
  this={tag()}
  class="error no-scrollbar"
  class:mobile={ui.isMobile}
  class:framed={tag() === 'div'}
  {@attach scrollbar}
>
  <!-- Centered with `margin: auto`, so that content that is too tall is not cut off at the top -->
  <div class="inner">
    <span class="kicker">{kicker}</span>
    <span class="tile"><Icon name={page.icon} /></span>
    <h1>{page.title()}</h1>
    <p class="text">{page.text()}</p>
    <div class="actions">
      {#if page.retry}
        <button
          type="button"
          class="primary"
          aria-disabled={busy ? 'true' : 'false'}
          style:cursor={busy ? 'progress' : 'pointer'}
          onclick={() => void run()}
        >
          <span class="spin" style:transform="rotate({rot}deg)">
            <Icon name={busy ? 'ti-loader-2' : 'ti-refresh'} />
          </span>{busy ? t.retrying() : t.retry()}
        </button>
        <button type="button" class="secondary" onclick={home}>{t.home()}</button>
      {:else}
        <button type="button" class="primary lift" onclick={home}>
          <span class="bi"><Icon name="ti-home" /></span>{t.home()}
        </button>
        <button type="button" class="secondary" onclick={back}>{t.back()}</button>
      {/if}
    </div>
    {#if support}
      <p class="support">
        {t.support_question()}
        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- mailto: to the organization -->
        <a href={mail}>{t.support()}</a>
      </p>
    {/if}
  </div>
</svelte:element>

<style>
  .error {
    flex: 1;
    min-height: 100dvh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    padding: 48px 32px;
    text-align: center;
  }

  .inner {
    margin: auto;
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  .error.mobile {
    padding: 28px 22px;
  }

  /* Inside the shell's card or panel ("Access denied"): as tall as the card, not the window */
  .error.framed {
    min-height: 0;
  }

  .kicker {
    height: 30px;
    padding: 0 13px;
    border-radius: 9999px;
    background: var(--surface2);
    display: grid;
    place-items: center;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 12.5px;
    font-weight: 700;
    color: var(--mfg);
  }

  .tile {
    margin-top: 22px;
    width: 96px;
    height: 96px;
    flex-shrink: 0;
    border-radius: 32px;
    background: var(--accsoft);
    color: var(--deep);
    display: grid;
    place-items: center;
    transform: rotate(-6deg);
    box-shadow: var(--sh1);
    font-size: 46px;
  }

  .mobile .tile {
    width: 84px;
    height: 84px;
    border-radius: 28px;
    font-size: 40px;
  }

  h1 {
    margin: 28px 0 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 38px;
    line-height: 1.15;
    text-wrap: balance;
  }

  .mobile h1 {
    margin-top: 24px;
    font-size: 30px;
  }

  .text {
    margin: 10px 0 0;
    max-width: 440px;
    font-size: 16px;
    line-height: 1.55;
    color: var(--mfg);
    text-wrap: pretty;
  }

  .mobile .text {
    font-size: 15px;
  }

  .actions {
    margin-top: 26px;
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    justify-content: center;
  }

  .mobile .actions {
    margin-top: 24px;
    width: 100%;
    max-width: 340px;
    flex-direction: column;
  }

  button {
    height: 44px;
    border-radius: 9999px;
    white-space: nowrap;
  }

  .mobile button {
    height: 48px;
    font-size: 15px;
  }

  .primary {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 9px;
    padding: 0 20px 0 16px;
    background: var(--ink);
    color: var(--inkfg);
    font-size: 14.5px;
    font-weight: 800;
    /* fixed value (shadow of the dark button) */
    box-shadow: 0 10px 24px -10px rgba(43, 34, 28, 0.5);
  }

  .primary.lift {
    gap: 8px;
    transition: transform var(--dur, 220ms) cubic-bezier(0.3, 1.5, 0.5, 1);
  }

  .error:not(.mobile) .primary.lift:hover {
    transform: translateY(-2px);
  }

  .mobile .primary {
    width: 100%;
  }

  .spin,
  .bi {
    display: grid;
    font-size: 18px;
  }

  .spin {
    transition: transform calc(var(--dur, 220ms) * 6.4) cubic-bezier(0.45, 0, 0.2, 1);
  }

  .secondary {
    padding: 0 18px;
    font-size: 14px;
    font-weight: 700;
    transition: background-color var(--dur, 220ms);
  }

  .error:not(.mobile) .secondary:hover,
  .mobile .secondary:active {
    background: var(--hov);
  }

  .support {
    margin: 22px 0 0;
    font-size: 14px;
    color: var(--mfg);
  }

  .support a {
    font-weight: 700;
  }
</style>
