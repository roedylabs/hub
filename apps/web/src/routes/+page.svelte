<script lang="ts">
  import { m } from '#lib/paraglide/messages.js';
</script>

<svelte:head>
  <title>{m.app_name()}</title>
</svelte:head>

<main>
  <h1>{m.app_name()}</h1>
  <p>{m.home_placeholder()}</p>
</main>

<style>
  main {
    max-width: 40rem;
    margin: 0 auto;
    padding: 4rem 1.5rem;
  }

  h1 {
    font-family: var(--font-serif);
    font-weight: 400;
  }

  p {
    color: var(--mfg);
  }
</style>
