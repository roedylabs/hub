<!-- Account › Accounts: the user's Stalwart accounts (`AccountsView`). -->
<script lang="ts">
  import { accountsNav } from '#lib/accounts-nav.ts';
  import AccountsView from '#lib/components/AccountsView.svelte';
</script>

<AccountsView adding={false} nav={accountsNav(null)} />
