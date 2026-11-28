<!--
  Test host (`page.test.ts`, `a11y.test.ts`): the page with its dialog
  (`ui.pageDialog`) rendered next to it, as the layout does with `bare` dialogs.

  Taken over from Roedy Mail (`apps/web/src/routes/settings/appearance/DialogHost.svelte`).
-->
<script lang="ts">
  import { ui } from '@roedy/ui';
  import Page from './+page.svelte';

  const dialog = $derived(ui.pageDialog?.() ?? null);
</script>

<Page />
{#if dialog}{@render dialog.content(ui.isMobile)}{/if}
