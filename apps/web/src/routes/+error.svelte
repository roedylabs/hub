<!--
  Error page of the router and of the gateway: an error the gateway answered a
  page load with (`gatewayErrorIn`, docs/app-contract.md › Errors), otherwise
  the router's own (unknown address 404, 500). "Try again" reloads the page.
-->
<script lang="ts">
  import { page } from '$app/state';
  import { errorCodeOf, ErrorView, gatewayErrorIn } from '@roedy/ui';
  import { session } from '#lib/state/session.svelte.ts';

  const code = $derived(gatewayErrorIn() ?? errorCodeOf(page.status));
</script>

<ErrorView {code} appName={session.appName} support={session.config?.org.email ?? null} />
