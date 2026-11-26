<!--
  Account › Language: the interface language as cards – "System" (follows
  the browser, `prefs.browserLang`, also after a change) and the languages,
  each named in itself. While "System" is chosen a note says that the hub's
  mails use the language of the last sign-in. Values in `#lib/language.ts`;
  saved at once via `prefs.set()` (sequenced, conflicts handled there; a
  failure toasts from the layout), and the page switches its language at once.
  The arrow keys only move the focus, Space and Enter choose (a new language
  re-renders the whole interface).

  Taken over from Roedy Mail (`apps/web/src/routes/settings/language/+page.svelte`)
  with the section "Language" only: the hub stores no region, time zone or
  formats. With a single section the tabs and the section heading are left
  out; the cards follow the page head.
-->
<script lang="ts">
  import { radiogroup } from '#lib/actions/radiogroup.ts';
  import Icon from '#lib/components/Icon.svelte';
  import { languageValues } from '#lib/language.ts';
  import { pageOf } from '#lib/settings-nav.ts';
  import { prefs } from '#lib/state/prefs.svelte.ts';
  import { session } from '#lib/state/session.svelte.ts';
  import { ui } from '#lib/state/ui.svelte.ts';

  const v = $derived(
    languageValues({ lang: prefs.values.lang, browserLang: prefs.browserLang, appName: session.appName }, (patch) =>
      prefs.set(patch),
    ),
  );
  const isMobile = $derived(ui.isMobile);
  const info = $derived(pageOf('/account/language'));
</script>

<div class="page" class:mobile={isMobile}>
  {#if !isMobile}
    <h1 class="title">{info?.label}</h1>
    <p class="lead">{info?.desc}</p>
  {:else}
    <h1 class="m-title">{info?.label}</h1>
    <p class="m-lead">{info?.desc}</p>
  {/if}

  <div role="radiogroup" aria-label={info?.label} use:radiogroup={{ select: false }} class="langs">
    {#each v.langOpts as l (l.id)}
      <button type="button" role="radio" aria-checked={l.on} class="lang" class:on={l.on} onclick={l.onClick}>
        <span class="tile" class:icon={!l.code}>
          {#if l.code}{l.code}{:else}<Icon name="ti-world" />{/if}
        </span>
        <span class="lang-text">
          <!-- "System" is a word of the interface, the languages are named in themselves (WCAG 3.1.2). -->
          <span class="lang-name" lang={l.code ? l.id : undefined}>{l.name}</span>
          {#if l.hasSub}<span class="lang-sub">{l.sub}</span>{/if}
        </span>
        <span class="check"><Icon name="ti-check" /></span>
      </button>
    {/each}
  </div>
  {#if v.systemNote}<p class="lang-note">{v.systemNote}</p>{/if}
</div>

<style>
  .title {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 40px;
    line-height: 1.1;
  }

  .lead {
    margin: 10px 0 0;
    color: var(--mfg);
    font-size: 16px;
  }

  .m-title {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 32px;
    line-height: 1.1;
  }

  .m-lead {
    margin: 8px 0 0;
    font-size: 15px;
    line-height: 1.5;
    color: var(--mfg);
  }

  /* Language */
  .langs {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 12px;
    margin-top: 28px;
  }

  .mobile .langs {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 22px;
  }

  .lang {
    display: flex;
    align-items: center;
    gap: 14px;
    min-height: 64px;
    padding: 10px 14px 10px 10px;
    border-radius: 22px;
    background: var(--surface2);
    text-align: left;
    transition:
      box-shadow var(--dur, 220ms),
      background-color var(--dur, 220ms),
      transform var(--dur, 220ms) cubic-bezier(0.3, 1.5, 0.5, 1);
  }

  .lang.on {
    background: var(--surface);
    box-shadow:
      0 0 0 3px var(--acc),
      var(--sh2);
  }

  .page:not(.mobile) .lang:hover {
    transform: translateY(-3px);
  }

  .mobile .lang {
    min-height: 60px;
    padding: 8px 16px 8px 8px;
    transition:
      box-shadow var(--dur, 220ms),
      background-color var(--dur, 220ms);
  }

  .tile {
    width: 44px;
    height: 44px;
    flex-shrink: 0;
    border-radius: 15px;
    background: var(--surface);
    color: var(--fg);
    display: grid;
    place-items: center;
    font-family: var(--font-serif);
    font-size: 15px;
    transition:
      background-color var(--dur, 220ms),
      transform var(--dur, 220ms) cubic-bezier(0.3, 1.5, 0.5, 1);
  }

  .tile.icon {
    font-size: 20px;
  }

  .lang.on .tile {
    background: var(--acc);
    color: var(--accink);
    transform: rotate(-6deg);
  }

  .lang-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .lang-name {
    font-size: 15.5px;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .lang-sub {
    font-size: 13px;
    color: var(--mfg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mobile .lang-name,
  .mobile .lang-sub {
    overflow: visible;
    white-space: normal;
  }

  .check {
    width: 24px;
    height: 24px;
    flex-shrink: 0;
    border-radius: 9999px;
    background: transparent;
    border: 2px solid var(--bdhi);
    display: grid;
    place-items: center;
    font-size: 14px;
    color: var(--inkfg);
    transition: background-color var(--dur, 220ms);
  }

  .check :global(svg) {
    opacity: 0;
  }

  .lang.on .check {
    background: var(--ink);
    border: 0;
  }

  .lang.on .check :global(svg) {
    opacity: 1;
  }

  .lang-note {
    margin: 12px 0 0;
    font-size: 13px;
    color: var(--mfg);
  }
</style>
