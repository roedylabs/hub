<!-- One Stalwart account (`AccountDetail`); the key recreates the page for another account. -->
<script lang="ts">
  import { page } from '$app/state';
  import { accountNav } from '#lib/accounts-nav.ts';
  import AccountDetail from '#lib/components/AccountDetail.svelte';
</script>

{#key page.params.id}<AccountDetail id={page.params.id ?? ''} nav={accountNav()} />{/key}
