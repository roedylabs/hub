<script lang="ts">
  import type { Snippet } from 'svelte';
  import '@fontsource-variable/nunito-sans/opsz.css';
  import '@fontsource/young-serif/400.css';
  import '../app.css';

  let { children }: { children: Snippet } = $props();
</script>

{@render children()}
