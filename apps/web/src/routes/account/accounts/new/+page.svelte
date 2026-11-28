<!--
  Add account: the same page as Account › Accounts with the form open. `?next=` (a checked path, e.g. of the app
  that sent the user over "No account yet") is where it leads once the account is there.
-->
<script lang="ts">
  import { page } from '$app/state';
  import { accountsNav } from '#lib/accounts-nav.ts';
  import AccountsView from '#lib/components/AccountsView.svelte';
  import { safeNext } from '#lib/next.ts';

  const next = safeNext(page.url.searchParams.get('next'), page.url.origin);
</script>

<AccountsView adding={true} {next} nav={accountsNav(next)} />
