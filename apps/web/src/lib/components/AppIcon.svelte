<!--
  An app's icon on a tile: the sanitized SVG from the hub as an image
  (`/api/v1/apps/<slug>/icon.svg?v=…`), decorative next to the app's name;
  while the app waits for pairing (no icon yet) a plug.
-->
<script lang="ts">
  import Icon from '#lib/components/Icon.svelte';

  interface Props {
    src: string | null;
    /** Edge of the tile in px; the image takes about two thirds. */
    size?: number;
  }

  let { src, size = 44 }: Props = $props();
</script>

<span class="tile" class:pending={!src} style:--size="{size}px" aria-hidden="true">
  {#if src}<img {src} alt="" />{:else}<Icon name="ti-plug" />{/if}
</span>

<style>
  .tile {
    width: var(--size);
    height: var(--size);
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border-radius: calc(var(--size) * 0.32);
    background: var(--surface);
    box-shadow: var(--sh1);
  }

  .tile.pending {
    background: var(--surface2);
    box-shadow: none;
    color: var(--mfg);
    font-size: calc(var(--size) * 0.45);
  }

  img {
    display: block;
    width: 66%;
    height: 66%;
  }
</style>
