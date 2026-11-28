<!--
  Toast: notice at the bottom center, with "Undo" for actions with deferred
  execution (`../undo.ts`). Single-line as long as the text fits in the
  window; longer ones wrap into at most two lines, on mobile three. The error
  toast shows a warning icon and announces itself as `alert`, the others as
  `status`. Swiping sideways or down closes it and sends a pending action
  immediately, a short drag springs back; only where the app passes `swipe`
  (in the shell on mobile and on tablets, `toastSwipe()`), not in the desktop
  layout. If the page shows the bar "Unsaved changes", it sits 12 px above it
  (`lift`, `toast-lift.svelte.ts`); the change glides with `--dur`.

  Taken over from Roedy Mail (`apps/web/src/lib/components/Toast.svelte`)
  with the toast as one prop instead of Mail's `ToastValues` and without the
  place above Mail's tab bar (an app with one passes it as `lift`).
-->
<script lang="ts">
  import Icon from './Icon.svelte';
  import { texts } from '../i18n.ts';
  import { ui } from '../state/ui.svelte.ts';
  import { swipeToast, type ToastSwipe } from '../swipe.ts';
  import type { ToastItem } from '../undo.ts';

  const t = texts({
    en: { undo: 'Undo' },
    de: { undo: 'Rückgängig' },
  });

  interface Props {
    toast: ToastItem;
    isMobile: boolean;
    /** Distance from the bottom edge above the bar "Unsaved changes" in px; `null` at its usual place. */
    lift?: number | null;
    /** "Undo"; without it `ui.toasts.undo()`. */
    onundo?: () => void;
    /** Swiping the toast away (`toastSwipe()`); `null` where it cannot be swiped (desktop layout, pages without a shell). */
    swipe?: ToastSwipe | null;
  }

  let { toast, isMobile, lift = null, onundo = () => ui.toasts.undo(), swipe = null }: Props = $props();
</script>

<div
  class="toast"
  class:mobile={isMobile}
  role={toast.error ? 'alert' : 'status'}
  data-g="toast"
  style:bottom={lift === null ? undefined : `${lift}px`}
  {@attach swipeToast(() => swipe)}
>
  {#if toast.error}
    <span class="tile err"><Icon name="ti-alert-triangle" /></span>
  {:else}
    <span class="tile"><Icon name="ti-check" /></span>
  {/if}
  <span class="text">{toast.text}</span>
  {#if toast.undo && !toast.error}
    <button type="button" class="undo" onclick={onundo}>{t.undo()}</button>
  {:else}
    <span class="end"></span>
  {/if}
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
    touch-action: none;
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

  .undo {
    height: 34px;
    flex-shrink: 0;
    padding: 0 14px;
    border-radius: 9999px;
    font-size: 14px;
    font-weight: 800;
    /* light the accent, dark a dark accent on the light pill */
    color: var(--toastacc);
  }

  .undo:hover {
    background: color-mix(in oklab, var(--inkfg) 8%, transparent);
  }

  .mobile .undo {
    height: 32px;
    padding: 0 12px;
    font-size: 13.5px;
  }

  .mobile .undo:hover {
    background: none;
  }

  .end {
    width: 8px;
    flex-shrink: 0;
  }
</style>
