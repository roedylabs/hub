<!--
  App icon in four kinds (default mark, initial, Tabler icon, image). Fills
  the surrounding frame and inherits its colors from there.

  Taken over from Roedy Mail (`apps/web/src/lib/components/Logo.svelte`) with
  the hub's own default mark (four app tiles) in place of Mail's letters.
-->
<script lang="ts">
  import Icon from './Icon.svelte';
  import { isIconName } from '../icons.ts';
  import type { LogoValues } from '../logo.ts';

  let { logo = { isMark: true, text: 'R', fs: '50cqmin' } }: { logo?: LogoValues } = $props();

  const icon = $derived(logo.icon && isIconName(logo.icon) ? logo.icon : undefined);
</script>

<div class="logo">
  {#if logo.isMark}
    <!-- The hub's own mark: four app tiles, the first in full ink, the others at 55 % -->
    <svg class="mark" viewBox="0 0 48 48" fill="currentColor" aria-hidden="true">
      <rect x="8" y="8" width="14" height="14" rx="4" />
      <rect x="26" y="8" width="14" height="14" rx="4" opacity=".55" />
      <rect x="8" y="26" width="14" height="14" rx="4" opacity=".55" />
      <rect x="26" y="26" width="14" height="14" rx="4" opacity=".55" />
    </svg>
  {/if}
  {#if logo.isInit}
    <span class="initial" style:font-size={logo.fs}>{logo.text}</span>
  {/if}
  {#if logo.isIcon && icon}
    <!-- Size via CSS: container units are not valid everywhere as an SVG attribute -->
    <span class="glyph" style:width={logo.iconSize} style:height={logo.iconSize}><Icon name={icon} size="100%" /></span>
  {/if}
  {#if logo.isFill}
    <img class="fill" src={logo.src} alt="" draggable="false" />
  {/if}
  {#if logo.isPad}
    <img class="pad" src={logo.src} alt="" draggable="false" />
  {/if}
</div>

<style>
  .logo {
    width: 100%;
    height: 100%;
    container-type: size;
    display: grid;
    place-items: center;
  }

  svg {
    display: block;
    width: 64%;
    height: 64%;
    overflow: visible;
  }

  .glyph {
    display: grid;
    width: 56cqmin;
    height: 56cqmin;
  }

  .initial {
    font-family: var(--font-serif);
    line-height: 1;
    white-space: nowrap;
  }

  img {
    display: block;
  }

  .fill {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .pad {
    width: 72%;
    height: 72%;
    object-fit: contain;
  }
</style>
