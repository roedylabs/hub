<!--
  Tile of a Stalwart account: the address's first letter in the account's
  colour (`MA_COLORS`), as every Roedy app shows accounts (`stalwart.ts`).
  Decorative by default; with `label` it is an image with that name (e.g. the
  address where the tile stands alone). `size` in px, the corner radius and
  the letter scale with it; `tilt` turns it like the shell's tiles.
-->
<script lang="ts">
  import { accountColor, accountLetter } from '../stalwart.ts';

  let {
    email,
    color,
    size = 40,
    tilt = false,
    label,
  }: {
    email: string;
    /** Index into `MA_COLORS`. */
    color: number;
    size?: number;
    tilt?: boolean;
    /** Accessible name; without it the tile is hidden from assistive technology. */
    label?: string;
  } = $props();

  const c = $derived(accountColor(color));
</script>

<span
  class="tile"
  class:tilt
  style:--size="{size}px"
  style:background={c.bg}
  style:color={c.ink}
  role={label ? 'img' : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : 'true'}
>
  {accountLetter(email)}
</span>

<style>
  .tile {
    width: var(--size);
    height: var(--size);
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border-radius: calc(var(--size) * 0.34);
    font-size: calc(var(--size) * 0.45);
    font-weight: 800;
    line-height: 1;
    user-select: none;
  }

  .tilt {
    transform: rotate(-6deg);
    box-shadow: var(--sh1);
  }
</style>
