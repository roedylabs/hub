<!--
  Home page: greeting, the hint for administrators while apps wait for
  pairing, and the signed-in user's apps as tiles (icon, name, description,
  "Offline") that open the app; without any an empty state, for
  administrators with the way to Administration › Apps. The layout puts the
  shell's header with the account menu above it. Flow and values in
  `#lib/home.svelte.ts`.

  No design yet (docs/plan.md › Web app): built plainly with the tokens.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { Icon, LoadCard, ui } from '@roedy/ui';
  import { HomePage, homeValues } from '#lib/home.svelte.ts';
  import { m } from '#lib/paraglide/messages.js';

  const p = new HomePage();
  const v = $derived(homeValues(p, { openApps: () => void goto(resolve('/admin/apps')) }));
  const isMobile = $derived(ui.isMobile);
  const uid = $props.id();
  /** Tiles with the ids of what describes the link: the description and "Offline" (own prefixes: slugs cannot collide). */
  const tiles = $derived(
    v.tiles.map((t) => ({
      ...t,
      described: t.offline ? `${uid}-desc-${t.slug} ${uid}-off-${t.slug}` : `${uid}-desc-${t.slug}`,
    })),
  );

  onMount(() => {
    p.start();
    const visible = () => {
      if (document.visibilityState === 'visible') p.refresh();
    };
    document.addEventListener('visibilitychange', visible);
    return () => {
      document.removeEventListener('visibilitychange', visible);
      p.dispose();
    };
  });
</script>

<div class="page" class:mobile={isMobile}>
  <h1>{v.greeting}</h1>
  <p class="lead">{v.lead}</p>

  {#if v.pendingHint}
    <div role="note" class="hint">
      <span class="hint-icon"><Icon name="ti-plug" /></span>
      <span class="hint-text">{v.pendingHint.text}</span>
      <button type="button" class="hint-go" onclick={v.pendingHint.onClick}>
        {v.pendingHint.label}<Icon name="ti-arrow-right" />
      </button>
    </div>
  {/if}

  {#if p.status === 'loading'}
    {#if p.loading.slow}
      <div class="tiles" role="status" aria-label={m.loading_data()}>
        {#each [0, 1, 2] as i (i)}<span class="tile ph"></span>{/each}
      </div>
    {/if}
  {:else if p.status === 'failed'}
    <div class="card-space">
      <LoadCard title={m.home_load_failed()} text={p.failure} onRetry={() => void p.retry()} />
    </div>
  {:else if v.empty}
    <div class="card-space">
      <LoadCard icon="ti-apps" title={v.emptyTitle} text={v.emptyText} />
      {#if v.emptyAction}
        <button type="button" class="empty-go" onclick={v.emptyAction.onClick}>
          <Icon name="ti-settings" />{v.emptyAction.label}
        </button>
      {/if}
    </div>
  {:else}
    <h2 class="sr-only">{m.home_apps()}</h2>
    <ul class="tiles">
      {#each tiles as t (t.slug)}
        <!-- The name is the link; it covers the whole tile (`::after`), the rest describes it. -->
        <li class="tile" class:offline={t.offline}>
          <span class="icon"><img src={t.icon} alt="" width="40" height="40" /></span>
          <span class="text">
            <!-- eslint-disable svelte/no-navigation-without-resolve -- the app's own path, outside the hub's routes -->
            <a class="name" href={t.url} aria-describedby={t.described}>{t.name}</a>
            <!-- eslint-enable svelte/no-navigation-without-resolve -->
            <span class="desc" id="{uid}-desc-{t.slug}">{t.desc}</span>
          </span>
          {#if t.offline}
            <span class="off" id="{uid}-off-{t.slug}"><Icon name="ti-plug-connected-x" />{m.home_tile_offline()}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .page {
    width: 100%;
    max-width: 1080px;
    margin: 0 auto;
    padding: 36px 40px 96px;
  }

  .page.mobile {
    padding: 14px 18px 48px;
  }

  h1 {
    margin: 0;
    font-family: var(--font-serif);
    font-weight: 400;
    font-size: 40px;
    line-height: 1.1;
  }

  .mobile h1 {
    font-size: 32px;
  }

  .lead {
    margin: 10px 0 0;
    font-size: 16px;
    color: var(--mfg);
  }

  .mobile .lead {
    font-size: 15px;
  }

  /* Hint for administrators: apps wait for pairing */
  .hint {
    margin-top: 28px;
    display: flex;
    align-items: center;
    gap: 12px 16px;
    flex-wrap: wrap;
    padding: 14px 14px 14px 20px;
    border-radius: 24px;
    background: var(--accsoft);
  }

  .hint-icon {
    display: grid;
    font-size: 22px;
    color: var(--deep);
  }

  .hint-text {
    flex: 1 1 240px;
    min-width: 0;
    font-size: 14.5px;
    line-height: 1.5;
  }

  .hint-go,
  .empty-go {
    height: 40px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 0 18px;
    border-radius: 9999px;
    background: var(--ink);
    color: var(--inkfg);
    font-size: 14px;
    font-weight: 800;
    white-space: nowrap;
  }

  .card-space {
    margin-top: 32px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 18px;
  }

  .card-space > :global(*:first-child) {
    align-self: stretch;
  }

  /* Tiles */
  .tiles {
    margin: 32px 0 0;
    padding: 0;
    list-style: none;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
    gap: 18px;
  }

  .mobile .tiles {
    margin-top: 24px;
    grid-template-columns: 1fr;
    gap: 10px;
  }

  .tile {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 22px;
    border-radius: 28px;
    background: var(--surface);
    box-shadow: var(--sh1);
    color: inherit;
    text-decoration: none;
    transition:
      box-shadow var(--dur, 220ms),
      transform var(--dur, 220ms);
  }

  .mobile .tile {
    flex-direction: row;
    align-items: center;
    gap: 14px;
    padding: 14px 16px;
    border-radius: 24px;
  }

  li.tile:hover {
    box-shadow: var(--sh2);
    transform: translateY(-2px);
  }

  li.tile:has(.name:focus-visible) {
    box-shadow: 0 0 0 3px var(--acc);
  }

  .name {
    color: inherit;
    text-decoration: none;
    outline: none;
  }

  /* The link covers the tile */
  .name::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
  }

  .tile.ph {
    display: block;
    min-height: 168px;
    background: var(--surface2);
    box-shadow: none;
  }

  .mobile .tile.ph {
    min-height: 76px;
  }

  .icon {
    width: 56px;
    height: 56px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border-radius: 18px;
    background: var(--surface2);
  }

  .mobile .icon {
    width: 48px;
    height: 48px;
    border-radius: 16px;
  }

  .icon img {
    display: block;
    width: 36px;
    height: 36px;
  }

  .mobile .icon img {
    width: 30px;
    height: 30px;
  }

  .offline .icon img {
    opacity: 0.55;
  }

  .text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .name {
    font-size: 17px;
    font-weight: 800;
  }

  .desc {
    font-size: 14px;
    line-height: 1.45;
    color: var(--mfg);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .off {
    position: absolute;
    top: 18px;
    right: 18px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 12px;
    border-radius: 9999px;
    background: var(--surface2);
    color: var(--danger);
    font-size: 12.5px;
    font-weight: 800;
  }

  .mobile .off {
    position: static;
    margin-left: auto;
    flex-shrink: 0;
  }
</style>
