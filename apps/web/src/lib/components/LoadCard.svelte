<!--
  Card below the head of a settings page: rotated tile, heading and text.
  With `onRetry` loading failed: tile in `--danger`, `role="alert"` and "Try
  again"; otherwise an empty state or "not available". Presentational only.

  Taken over from Roedy Mail (`apps/web/src/lib/components/MailLoadCard.svelte`),
  trimmed to "Try again": no "Sign in again" and "Go to connection" of the
  mail accounts.
-->
<script lang="ts">
  import Icon from '#lib/components/Icon.svelte';
  import type { IconName } from '#lib/icons.ts';
  import { m } from '#lib/paraglide/messages.js';

  interface Props {
    /** Icon of the tile; for a failed load `ti-cloud-off`. */
    icon?: IconName;
    title: string;
    text: string;
    /** Loading failed: "Try again" runs it again. */
    onRetry?: (() => void) | null;
  }

  let { icon = 'ti-cloud-off', title, text, onRetry = null }: Props = $props();
</script>

<div class="card" class:fail={!!onRetry} role={onRetry ? 'alert' : undefined}>
  <span class="tile"><Icon name={icon} /></span>
  <div class="title">{title}</div>
  <div class="text">{text}</div>
  {#if onRetry}
    <div class="actions">
      <button type="button" class="btn retry" onclick={onRetry}><Icon name="ti-refresh" />{m.error_retry()}</button>
    </div>
  {/if}
</div>

<style>
  .card {
    margin-top: var(--load-card-top, 28px);
    padding: 40px 24px;
    border-radius: 26px;
    background: var(--surface2);
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }

  .card.fail {
    animation: snFade 200ms ease-out;
  }

  .tile {
    width: 56px;
    height: 56px;
    border-radius: 18px;
    background: var(--surface);
    box-shadow: var(--sh1);
    display: grid;
    place-items: center;
    transform: rotate(-6deg);
    font-size: 26px;
    color: var(--mfg);
  }

  .fail .tile {
    color: var(--danger);
  }

  .title {
    margin-top: 16px;
    font-family: var(--font-serif);
    font-size: 22px;
    line-height: 1.2;
    text-wrap: balance;
  }

  .text {
    margin-top: 8px;
    max-width: 440px;
    font-size: 14.5px;
    line-height: 1.5;
    color: var(--mfg);
    text-wrap: pretty;
    overflow-wrap: anywhere;
  }

  .actions {
    margin-top: 20px;
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }

  .btn {
    height: 44px;
    display: flex;
    align-items: center;
    gap: 8px;
    border-radius: 9999px;
    background: var(--surface);
    font-size: 14px;
    font-weight: 700;
    white-space: nowrap;
    box-shadow: var(--sh1);
    transition: background-color var(--dur, 220ms);
  }

  .btn :global(svg) {
    font-size: 17px;
  }

  .retry {
    padding: 0 18px 0 14px;
  }

  .retry:hover {
    background: var(--sunken);
  }
</style>
