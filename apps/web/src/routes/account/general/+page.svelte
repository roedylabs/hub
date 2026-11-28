<!--
  Account › General: behaviour (ask before deleting, undo delay, keyboard
  shortcuts – the last only on the desktop, single keys do not exist on
  mobile) and developer mode, with section tabs. They apply in the hub and in
  every app (the shared settings). Values in `#lib/general.ts`; saved at once
  via `prefs.set()` (sequenced, conflicts handled there; a failure toasts
  from the layout). Switch and segment styles are classes derived from `on`.

  Taken over from Roedy Mail (`apps/web/src/routes/settings/general/+page.svelte`)
  with the behaviour section only and developer mode.
-->
<script lang="ts">
  import { radiogroup, ui } from '@roedy/ui';
  import { generalValues, type GeneralContext } from '#lib/general.ts';
  import { m } from '#lib/paraglide/messages.js';
  import { scrollToSection } from '#lib/sections.ts';
  import { pageOf } from '#lib/settings-nav.ts';
  import { prefs } from '#lib/state/prefs.svelte.ts';

  let root: HTMLElement | undefined = $state();
  const ctx: GeneralContext = {
    set: (patch) => prefs.set(patch),
    scrollTo: (section) => scrollToSection(root, section, ui.motion),
  };
  const v = $derived(generalValues({ values: prefs.values }, ctx));
  const isMobile = $derived(ui.isMobile);
  const info = $derived(pageOf('/account/general'));
</script>

{#snippet rowText(label: string, desc: string)}
  <div class="row-text">
    <div class="row-label">{label}</div>
    <div class="row-desc">{desc}</div>
  </div>
{/snippet}

{#snippet toggleRow(label: string, desc: string, t: { on: boolean; onClick: () => void })}
  <div class="row wrap toggle">
    {@render rowText(label, desc)}
    <button
      type="button"
      role="switch"
      class="switch"
      class:on={t.on}
      aria-checked={t.on}
      aria-label={label}
      onclick={t.onClick}
    >
      <span class="knob"></span>
    </button>
  </div>
{/snippet}

<div bind:this={root} class="page" class:mobile={isMobile}>
  {#if !isMobile}
    <h1 class="title">{info?.label}</h1>
    <p class="lead">{info?.desc}</p>
  {:else}
    <h1 class="m-title">{info?.label}</h1>
    <p class="m-lead">{info?.desc}</p>
  {/if}

  <div class="tabs no-scrollbar">
    {#each v.genTabs as t (t.label)}<button type="button" class="tab" onclick={t.onClick}>{t.label}</button>{/each}
  </div>

  <section data-sec="behavior" class="sec first">
    <h2 class="sec-title">{m.general_tab_behavior()}</h2>
    <p class="sec-desc">{m.general_behavior_desc()}</p>
    <div class="box">
      {@render toggleRow(
        m.general_confirm(),
        isMobile ? m.general_confirm_desc_mobile() : m.general_confirm_desc(),
        v.tgConfirm,
      )}
      <div class="sep"></div>
      <div class="row wrap">
        {@render rowText(m.general_undo(), m.general_undo_desc())}
        <div role="radiogroup" use:radiogroup aria-label={m.general_undo()} class="seg undo">
          {#each v.undoOpts as o (o.value)}
            <button type="button" role="radio" aria-checked={o.on} class:on={o.on} onclick={o.onClick}>
              {o.label}
            </button>
          {/each}
        </div>
      </div>
      {#if !isMobile}
        <!-- Single keys do not exist on mobile: the row is missing there. -->
        <div class="sep"></div>
        {@render toggleRow(m.general_keys(), m.general_keys_desc(), v.tgKeys)}
      {/if}
    </div>
  </section>

  <section data-sec="developer" class="sec">
    <h2 class="sec-title">{m.general_tab_developer()}</h2>
    <p class="sec-desc">{m.general_developer_desc()}</p>
    <div class="box">
      {@render toggleRow(m.general_developer(), m.general_developer_row_desc(), v.tgDeveloper)}
    </div>
  </section>
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

  /* Tabs */
  .tabs {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 22px;
  }

  .mobile .tabs {
    flex-wrap: nowrap;
    margin: 20px -18px 0;
    padding: 0 18px;
    overflow-x: auto;
  }

  .tab {
    height: 40px;
    padding: 0 18px;
    border-radius: 9999px;
    background: var(--surface2);
    font-size: 14px;
    font-weight: 700;
    white-space: nowrap;
    transition: background-color var(--dur, 220ms);
  }

  .tab:hover {
    background: var(--sunken);
  }

  .mobile .tab {
    height: 38px;
    flex-shrink: 0;
    padding: 0 16px;
    font-size: 13.5px;
    transition: none;
  }

  .mobile .tab:hover {
    background: var(--surface2);
  }

  .mobile .tab:active {
    background: var(--sunken);
  }

  /* Section */
  .sec {
    margin-top: 44px;
  }

  .sec.first {
    margin-top: 36px;
  }

  .mobile .sec {
    margin-top: 34px;
  }

  .mobile .sec.first {
    margin-top: 28px;
  }

  .sec-title {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 24px;
  }

  .mobile .sec-title {
    font-size: 22px;
  }

  .sec-desc {
    margin: 6px 0 0;
    font-size: 14.5px;
    color: var(--mfg);
  }

  .mobile .sec-desc {
    margin: 5px 0 0;
    font-size: 14px;
    line-height: 1.5;
  }

  .box {
    margin-top: 18px;
    border-radius: 26px;
    background: var(--surface2);
    padding: 6px;
  }

  .mobile .box {
    margin-top: 16px;
    border-radius: 24px;
    padding: 4px;
  }

  .sep {
    height: 1px;
    margin: 0 18px;
    background: var(--bd);
  }

  .mobile .sep {
    margin: 0 14px;
  }

  /* Row with label and control */
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 16px 18px;
  }

  .row.wrap {
    gap: 12px 24px;
    flex-wrap: wrap;
  }

  .mobile .row.wrap {
    display: block;
    padding: 14px 14px 12px;
  }

  .mobile .row.toggle {
    display: flex;
    flex-wrap: nowrap;
    justify-content: flex-start;
    gap: 16px;
    padding: 14px;
  }

  .row-text {
    flex: 1 1 220px;
    min-width: 0;
  }

  .mobile .row.toggle .row-text {
    flex: 1;
  }

  .row-label {
    font-size: 15.5px;
    font-weight: 700;
  }

  .mobile .row-label {
    font-size: 15px;
  }

  .row-desc {
    margin-top: 3px;
    font-size: 14px;
    color: var(--mfg);
  }

  .mobile .row-desc {
    margin-top: 2px;
    font-size: 13.5px;
  }

  /* Segment */
  .seg {
    display: flex;
    gap: 2px;
    padding: 4px;
    border-radius: 9999px;
    background: var(--sunken);
  }

  .mobile .seg {
    margin-top: 12px;
  }

  .seg button {
    height: 36px;
    padding: 0 16px;
    border-radius: 9999px;
    background: transparent;
    color: var(--mfg);
    font-size: 14px;
    font-weight: 700;
    white-space: nowrap;
    transition:
      background-color var(--dur, 220ms),
      color var(--dur, 220ms);
  }

  .seg button:hover,
  .seg button.on {
    color: var(--fg);
  }

  .seg button.on {
    background: var(--surface);
  }

  .seg.undo button.on {
    box-shadow: var(--sh1);
  }

  .mobile .seg button {
    flex: 1;
    min-width: 0;
    height: 38px;
    padding: 0;
    font-size: 13.5px;
    transition: none;
  }

  .mobile .seg.undo button {
    height: 36px;
    transition:
      background-color var(--dur, 220ms),
      color var(--dur, 220ms);
  }

  .mobile .seg button:hover:not(.on) {
    color: var(--mfg);
  }

  /* Switch */
  .switch {
    width: 52px;
    height: 32px;
    border-radius: 9999px;
    background: var(--sunken);
    position: relative;
    flex-shrink: 0;
    transition: background-color var(--dur, 220ms);
  }

  .switch.on {
    background: var(--acc);
  }

  .knob {
    position: absolute;
    top: 4px;
    left: 4px;
    width: 24px;
    height: 24px;
    border-radius: 9999px;
    /* fixed values (switch) */
    background: #fff;
    box-shadow: 0 2px 4px rgba(60, 35, 10, 0.25);
    transition: transform var(--dur, 220ms) cubic-bezier(0.3, 1.5, 0.5, 1);
  }

  .switch.on .knob {
    transform: translateX(20px);
  }
</style>
