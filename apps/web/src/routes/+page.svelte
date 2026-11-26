<!--
  Home page: still the plain placeholder (the design follows, docs/plan.md ›
  Web app) with who is signed in and "Sign out".
-->
<script lang="ts">
  import { logout } from '#lib/logout.ts';
  import { m } from '#lib/paraglide/messages.js';
  import { session } from '#lib/state/session.svelte.ts';

  let busy = $state(false);

  async function signOut() {
    if (busy) return;
    busy = true;
    try {
      await logout();
    } finally {
      busy = false;
    }
  }
</script>

<main>
  <h1>{session.appName}</h1>
  <p>{m.home_placeholder()}</p>
  {#if session.user}
    <p class="who">{m.home_signed_in({ name: session.user.name })}</p>
    <button type="button" aria-disabled={busy ? 'true' : undefined} onclick={() => void signOut()}>
      {m.account_logout()}
    </button>
  {/if}
</main>

<style>
  main {
    width: 100%;
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

  .who {
    color: var(--fg);
  }

  button {
    height: 44px;
    padding: 0 20px;
    border-radius: 9999px;
    background: var(--ink);
    color: var(--inkfg);
    font-weight: 800;
  }
</style>
