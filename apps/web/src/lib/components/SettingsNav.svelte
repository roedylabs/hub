<!--
  Navigation of the account and administration area: on the desktop the bar
  left of the content card ("Back to home", the groups, at the bottom the
  hub's logo and name); on mobile the area's list, with its name as the
  heading.

  Taken over from Roedy Mail (`apps/web/src/lib/components/SettingsNav.svelte`)
  without the collapsed variant, "Advanced settings", the version and the
  mail accounts.
-->
<script lang="ts">
  import Icon from '#lib/components/Icon.svelte';
  import Logo from '#lib/components/Logo.svelte';
  import type { SettingsNavValues } from '#lib/settings-nav.ts';

  let { isMobile, appName, logo, backLabel, panelBack, mPanelH, settingsGroups }: SettingsNavValues = $props();
</script>

{#if !isMobile}
  <aside class="nav">
    <button type="button" class="back" onclick={panelBack}>
      <span class="back-icon"><Icon name="ti-arrow-left" /></span>
      <span class="back-label">{backLabel}</span>
    </button>
    <nav class="groups no-scrollbar" aria-label={mPanelH}>
      {#each settingsGroups as g, i (g.label || i)}
        <div class="group">
          {#if g.hasLabel}<div class="group-label">{g.label}</div>{/if}
          {#each g.items as it (it.id)}
            <button
              type="button"
              class="item"
              class:on={it.on}
              style:font-weight={it.fw}
              aria-current={it.on ? 'page' : undefined}
              onclick={it.onClick}
            >
              <span class="tile"><Icon name={it.icon} /></span>
              <span class="label">{it.label}</span>
            </button>
          {/each}
        </div>
      {/each}
    </nav>
    <div class="card">
      <div class="app">
        <span class="app-logo"><Logo {logo} /></span>
        <div class="app-name">{appName}</div>
      </div>
    </div>
  </aside>
{:else}
  <h1 class="m-title">{mPanelH}</h1>
  {#each settingsGroups as g, i (g.label || i)}
    <div class="m-group">
      {#if g.hasLabel}<div class="m-group-label">{g.label}</div>{/if}
      <div class="m-list">
        {#each g.items as it (it.id)}
          <button type="button" class="m-item" onclick={it.onClick}>
            <span class="m-tile"><Icon name={it.icon} /></span>
            <span class="m-label">{it.label}</span>
            <span class="m-chev"><Icon name="ti-chevron-right" /></span>
          </button>
        {/each}
      </div>
    </div>
  {/each}
  <div class="m-app">
    <span class="m-app-logo"><Logo {logo} /></span>
    <span class="m-app-text">{appName}</span>
  </div>
{/if}

<style>
  .nav {
    position: relative;
    z-index: 30;
    width: 270px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    margin-bottom: 16px;
    padding: 0 14px 0 16px;
    border-radius: 0 28px 28px 0;
    background: var(--cnv);
    overflow: hidden;
  }

  .back {
    height: 50px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 16px 0 8px;
    border-radius: 9999px;
    font-size: 15px;
    font-weight: 700;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    transition: background-color var(--dur, 220ms);
  }

  .back:hover {
    background: var(--hov);
  }

  .back-icon {
    width: 34px;
    height: 34px;
    border-radius: 9999px;
    background: var(--surface);
    box-shadow: var(--sh1);
    display: grid;
    place-items: center;
    flex-shrink: 0;
    font-size: 18px;
  }

  .back-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .groups {
    flex: 1;
    min-height: 0;
    overflow: hidden auto;
    padding: 12px 0;
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .group + .group {
    margin-top: 12px;
  }

  .group-label {
    height: 36px;
    padding: 0 14px 6px;
    display: flex;
    align-items: flex-end;
    overflow: hidden;
    white-space: nowrap;
    font-family: var(--font-serif);
    font-size: 15px;
    color: var(--mfg);
  }

  .item {
    height: 44px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 6px;
    border-radius: 16px;
    color: var(--mfg);
    font-size: 14.5px;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    transition: background-color var(--dur, 220ms);
  }

  .item:hover {
    background: var(--hov);
    color: var(--fg);
  }

  .item.on {
    background: var(--surface);
    box-shadow: var(--sh1);
    color: var(--fg);
  }

  .tile {
    width: 32px;
    height: 32px;
    border-radius: 11px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    font-size: 18px;
  }

  .on .tile {
    background: var(--acc);
    color: var(--accink);
  }

  .label {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .card {
    width: 240px;
    flex-shrink: 0;
    border-radius: 22px;
    background: var(--surface);
    box-shadow: var(--sh1);
    padding: 14px 16px;
  }

  .app {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .app-logo {
    overflow: hidden;
    width: 34px;
    height: 34px;
    border-radius: 12px;
    background: var(--logo-bg, var(--acc));
    color: var(--logo-ink, var(--accink));
    display: grid;
    place-items: center;
    flex-shrink: 0;
    transform: rotate(calc(var(--logo-tilt, 1) * -4deg));
  }

  .app-name {
    min-width: 0;
    flex: 1;
    font-size: 14px;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .m-title {
    margin: 0 4px;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 32px;
    line-height: 1.1;
  }

  .m-group {
    margin-top: 22px;
  }

  .m-group-label {
    padding: 0 8px 8px;
    font-family: var(--font-serif);
    font-size: 15px;
    color: var(--mfg);
  }

  .m-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-radius: 24px;
    background: var(--surface2);
    padding: 4px;
  }

  .m-item {
    height: 56px;
    display: flex;
    align-items: center;
    gap: 13px;
    padding: 0 12px 0 10px;
    border-radius: 20px;
    font-size: 15.5px;
    font-weight: 700;
    text-align: left;
    transition: background-color var(--dur, 220ms);
  }

  .m-item:hover {
    background: var(--hov);
  }

  .m-tile {
    width: 36px;
    height: 36px;
    flex-shrink: 0;
    border-radius: 12px;
    display: grid;
    place-items: center;
    background: var(--accsoft);
    color: var(--deep);
    font-size: 19px;
  }

  .m-label {
    flex: 1;
    min-width: 0;
  }

  .m-chev {
    font-size: 18px;
    color: var(--dim);
    display: grid;
  }

  .m-app {
    margin-top: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
  }

  .m-app-logo {
    overflow: hidden;
    width: 28px;
    height: 28px;
    border-radius: 10px;
    background: var(--logo-bg, var(--acc));
    color: var(--logo-ink, var(--accink));
    display: grid;
    place-items: center;
    transform: rotate(calc(var(--logo-tilt, 1) * -4deg));
  }

  .m-app-text {
    font-size: 13px;
    font-weight: 700;
    color: var(--mfg);
  }
</style>
