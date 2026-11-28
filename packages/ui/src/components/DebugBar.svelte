<!--
  Debug bar: layout, window size and view at the bottom left in the shell, as
  long as the app's debug mode is on (`debugBarValues`, `../developer.ts`).
  Takes no input and stays hidden from screen readers.

  Taken over from Roedy Mail (`apps/web/src/lib/components/DebugBar.svelte`)
  with the props `text` and `bottom` instead of Mail's `dbgText` and `dbgB`.
-->
<script lang="ts">
  import type { DebugBarValues } from '../developer.ts';
  import { texts } from '../i18n.ts';

  const t = texts({
    en: { badge: 'DEBUG' },
    de: { badge: 'DEBUG' },
  });

  let { text, bottom }: DebugBarValues = $props();
</script>

<div class="bar" aria-hidden="true" style:bottom>
  <span class="tag">{t.badge()}</span><span class="text">{text}</span>
</div>

<style>
  .bar {
    position: absolute;
    left: 12px;
    z-index: 97;
    max-width: calc(100% - 24px);
    pointer-events: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 12px 5px 5px;
    border-radius: 9999px;
    background: var(--ink);
    color: var(--inkfg);
    box-shadow: var(--sh2);
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 11.5px;
    font-weight: 600;
    opacity: 0.94;
  }

  .tag {
    flex-shrink: 0;
    padding: 3px 8px;
    border-radius: 9999px;
    background: var(--acc);
    color: var(--accink);
    font-weight: 800;
  }

  .text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
