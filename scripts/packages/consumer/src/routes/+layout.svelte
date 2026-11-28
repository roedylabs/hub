<script lang="ts">
  import '@roedy/tokens/tokens.css';
  import '@roedy/ui/base.css';
  import { connectSettings } from '@roedy/ui';
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();

  connectSettings({ lang: () => 'de', mode: () => 'dark' });
</script>

{@render children()}
