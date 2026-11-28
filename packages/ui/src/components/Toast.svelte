<!--
  Toast: notice at the bottom center. Single-line as long as the text fits in
  the window; longer ones wrap into at most two lines, on mobile three. The
  error toast shows a warning icon and announces itself as `alert`, the
  others as `status`. If the page shows the bar "Unsaved changes", it sits
  12 px above it (`lift`, `toast-lift.svelte.ts`); the change glides
  with `--dur`.

  Taken over from Roedy Mail (`apps/web/src/lib/components/Toast.svelte`)
  without "Undo" and swiping.
-->
<script lang="ts">
  import Icon from './Icon.svelte';
  import type { ToastItem } from '../state/ui.svelte.ts';

  interface Props {
    toast: ToastItem;
    isMobile: boolean;
    /** Distance from the bottom edge above the bar "Unsaved changes" in px; `null` at its usual place. */
    lift?: number | null;
  }

  let { toast, isMobile, lift = null }: Props = $props();
</script>

<div
  class="toast"
  class:mobile={isMobile}
  role={toast.error ? 'alert' : 'status'}
  style:bottom={lift === null ? undefined : `${lift}px`}
>
  {#if toast.error}
    <span class="tile err"><Icon name="ti-alert-triangle" /></span>
  {:else}
    <span class="tile"><Icon name="ti-check" /></span>
  {/if}
  <span class="text">{toast.text}</span>
  <span class="end"></span>
</div>

<style>
  .toast {
    position: absolute;
    left: 50%;
    bottom: 32px;
    z-index: 95;
    /* above the bar "Unsaved changes" and back; without motion --dur is 0 */
    transition: bottom var(--dur, 220ms) ease;
    /* like the end of snToast, so that the toast is centered without animations too */
    transform: translateX(-50%);
    /* as wide as the text, at most the window: long texts (e.g. server names) wrap */
    width: max-content;
    max-width: calc(100% - 32px);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px;
    border-radius: 9999px;
    background: var(--ink);
    color: var(--inkfg);
    box-shadow: 0 20px 40px -12px rgba(43, 34, 28, 0.5);
    animation: snToast 420ms cubic-bezier(0.2, 1.4, 0.3, 1) both;
  }

  /* on mobile at the bottom, above the safe area */
  .toast.mobile {
    bottom: 14px;
    max-width: calc(100% - 24px);
    gap: 10px;
    padding: 7px;
  }

  .tile {
    width: 34px;
    height: 34px;
    flex-shrink: 0;
    border-radius: 9999px;
    background: var(--acc);
    color: var(--accink);
    display: grid;
    place-items: center;
    font-size: 18px;
  }

  .tile.err {
    background: var(--danger);
    color: var(--inkfg);
  }

  .mobile .tile {
    width: 32px;
    height: 32px;
    font-size: 17px;
  }

  .text {
    min-width: 0;
    font-size: 15px;
    font-weight: 700;
    line-height: 1.3;
    overflow-wrap: anywhere;
    text-wrap: pretty;
    padding-right: 4px;
    /* at most two lines */
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
  }

  /* on mobile up to three lines */
  .mobile .text {
    font-size: 14.5px;
    padding-right: 0;
    -webkit-line-clamp: 3;
    line-clamp: 3;
  }

  .end {
    width: 8px;
    flex-shrink: 0;
  }
</style>
