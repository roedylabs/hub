<!--
  "No account yet" (`/account/no-account?next=<path>`): where an app sends a
  signed-in user without a Stalwart account, in the design of the hub's
  sign-in pages. Names the app, what the user needs, "Add account" (with the
  same `next`) and "Back to home". A user with accounts goes straight on to
  `next`; until that is known the page stays empty. Flow in
  `#lib/no-account.svelte.ts`.

  Taken over from Roedy Mail (`apps/web/src/routes/no-account/+page.svelte`)
  as the hub's page for every app.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import AuthResult from '#lib/components/auth/AuthResult.svelte';
  import { leaveTo } from '#lib/navigate.ts';
  import { nextOf } from '#lib/next.ts';
  import { NoAccountPage } from '#lib/no-account.svelte.ts';
  import { m } from '#lib/paraglide/messages.js';
  import { session } from '#lib/state/session.svelte.ts';

  const p = new NoAccountPage(nextOf(page.url), { leave: (to) => leaveTo(to) });
  const reg = () => () => {};
  // eslint-disable-next-line svelte/no-navigation-without-resolve -- `/account/accounts/new` with the checked `next`
  const add = () => void goto(p.addHref);

  onMount(() => {
    void p.load();
    return () => p.dispose();
  });
</script>

{#snippet text()}{p.app ? m.noacct_text_app({ app: p.app }) : m.noacct_text()}{/snippet}

{#if p.ready}
  <AuthResult
    docTitle={m.noacct_doc_title({ app: session.appName })}
    title={m.noacct_title()}
    themeToggle
    {reg}
    icon="ti-mail-plus"
    role={null}
    {text}
    chips={{
      label: m.noacct_need(),
      always: true,
      items: [
        { icon: 'ti-at', text: m.noacct_email() },
        { icon: 'ti-key', text: m.noacct_password() },
        { icon: 'ti-server', text: m.noacct_server() },
      ],
    }}
    primary={{
      label: m.acct_add(),
      icon: 'ti-arrow-right',
      onclick: add,
    }}
    quiet={{ label: m.noacct_home(), icon: 'ti-arrow-left', onclick: () => void goto(resolve('/')) }}
  />
{/if}
