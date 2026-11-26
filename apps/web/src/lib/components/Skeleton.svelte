<!--
  Skeleton of a settings page: placeholder while data without a cache takes
  longer than 150 ms to load (`Loading` in ui.svelte.ts), as an overview
  (card and tiles, `set="ueb"`) or a list (`set="list"`), with a hint below.

  Taken over from Roedy Mail (`apps/web/src/lib/components/Skeleton.svelte`),
  trimmed to the settings kind (no mailbox, message, start page or search),
  hence without `kind`.
-->
<script lang="ts">
  import Icon from '#lib/components/Icon.svelte';
  import { m } from '#lib/paraglide/messages.js';
  import { skeletonValues, type SkeletonSet } from '#lib/shell.ts';

  interface Props {
    set?: SkeletonSet;
    isMobile?: boolean;
    /** Hint below the skeleton; without a value "Loading data …". */
    note?: string;
  }

  let { set = 'list', isMobile = false, note }: Props = $props();

  const v = $derived(skeletonValues(set));
</script>

<!-- One bone: surface with width, height, corners; `sunk` on a tinted surface, `still` without shimmer. -->
{#snippet b(w: string, h: string, r: string, o: { d?: string; sunk?: boolean; still?: boolean; cls?: string } = {})}
  <span
    class="bone {o.cls ?? ''}"
    class:sunk={o.sunk}
    class:still={o.still}
    style:width={w}
    style:height={h}
    style:border-radius={r}
    style:animation-delay={o.d}
  ></span>
{/snippet}

{#snippet setLoading()}
  <div class="set-note"><span class="spin"><Icon name="ti-loader-2" /></span>{note ?? m.loading_data()}</div>
{/snippet}

{#if !isMobile}
  <div class="set" role="status" aria-label={m.loading_server()}>
    {#if v.skSetUeb}
      <div class="ueb-card">
        {@render b('56px', '56px', '19px', { sunk: true, cls: 'fix' })}
        <div class="lines10">
          {@render b('44%', '20px', '8px', { sunk: true })}{@render b('64%', '12px', '6px', { sunk: true })}
        </div>
      </div>
      <div class="ueb-grid">
        {#each ['0ms', '150ms', '300ms'] as d (d)}{@render b('auto', '112px', '24px', { d })}{/each}
      </div>
    {/if}
    {#if v.skSetList}
      <div class="list-box">
        {#each v.skRows3 as r (r.d)}
          <div>
            {#if r.sep}<div class="sep"></div>{/if}
            <div class="list-row">
              {@render b('42px', '42px', '14px', { d: r.d, sunk: true, cls: 'fix' })}
              <div class="lines9 grow">
                {@render b(r.a, '13px', '7px', { d: r.d, sunk: true })}{@render b(r.b, '11px', '6px', {
                  d: r.d,
                  sunk: true,
                })}
              </div>
            </div>
          </div>
        {/each}
      </div>
    {/if}
    {@render setLoading()}
  </div>
{:else}
  <div class="set m-set" role="status" aria-label={m.loading_server()}>
    {#if v.skSetUeb}
      {@render b('auto', '120px', '24px', { cls: 'block' })}
      <div class="m-ueb-grid">
        {@render b('auto', '96px', '22px', { d: '150ms' })}{@render b('auto', '96px', '22px', { d: '300ms' })}
      </div>
    {/if}
    {#if v.skSetList}
      <div class="list-box m-list-box">
        {#each v.skRows3 as r (r.d)}
          <div class="m-list-row">
            {@render b('40px', '40px', '14px', { d: r.d, sunk: true, cls: 'fix' })}
            <div class="lines8 grow">
              {@render b(r.a, '12px', '6px', { sunk: true, still: true })}{@render b(r.b, '10px', '5px', {
                sunk: true,
                still: true,
              })}
            </div>
          </div>
        {/each}
      </div>
    {/if}
    {@render setLoading()}
  </div>
{/if}

<style>
  .bone {
    display: inline-block;
    flex-shrink: 1;
    background: var(--surface2);
    animation: var(--shim, none);
  }

  .bone.sunk {
    background: var(--sunken);
  }

  .bone.still {
    animation: none;
  }

  .block {
    display: block;
  }

  .fix {
    flex-shrink: 0;
  }

  .grow {
    flex: 1;
  }

  .lines8,
  .lines9,
  .lines10 {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .lines8 {
    gap: 8px;
  }

  .lines9 {
    gap: 9px;
  }

  .lines10 {
    gap: 10px;
  }

  .set {
    margin-top: 28px;
  }

  .ueb-card {
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 22px;
    border-radius: 26px;
    background: var(--surface2);
  }

  .ueb-grid {
    margin-top: 12px;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 12px;
  }

  .ueb-grid :global(.bone),
  .m-ueb-grid :global(.bone) {
    display: block;
  }

  .list-box {
    border-radius: 26px;
    background: var(--surface2);
    padding: 6px;
  }

  .sep {
    height: 1px;
    background: var(--bd);
    margin: 0 18px;
  }

  .list-row {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 14px 16px 14px 12px;
  }

  .set-note {
    margin-top: 14px;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13.5px;
    font-weight: 700;
    color: var(--mfg);
  }

  .spin {
    display: grid;
    font-size: 16px;
    animation: var(--spin, none);
  }

  .m-set {
    margin-top: 22px;
  }

  .m-ueb-grid {
    margin-top: 10px;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }

  .m-list-box {
    border-radius: 24px;
    padding: 4px;
  }

  .m-list-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 10px;
  }

  .m-set .set-note {
    margin-top: 12px;
  }
</style>
