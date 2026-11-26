<!--
  Shared page of the routes `/login`, `/login/forgot` and
  `/login/reset/[token]`: the page persists when switching between them –
  one screen with steps, inputs and lockouts are kept –, the address selects
  the step (`fgOpen`, `pwNewOpen`) and follows it (`onFgView`, `onPwNewLeave`).

  Taken over from Roedy Mail (`apps/web/src/routes/(auth)/+layout.svelte`)
  with the hub's routes under `/login`.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import LoginPage from '#lib/components/LoginPage.svelte';
  import type { LoginRoute } from '#lib/login.svelte.ts';
  import type { LayoutProps } from './$types';

  let { children }: LayoutProps = $props();

  const route = $derived.by((): LoginRoute => {
    const path = page.url.pathname;
    if (path === '/login/forgot') return { kind: 'forgot' };
    if (path.startsWith('/login/reset/')) return { kind: 'newpw', token: page.params.token ?? '' };
    return { kind: 'login' };
  });

  function navigate(to: 'login' | 'forgot') {
    // Without `reset`: the focus stays in the field of the new step (otherwise SvelteKit moves it to <body>).
    // The query (`next`, `reauth`) goes along, so that the sign-in after "Reset password" still leads there.
    const query = page.url.search;
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve() plus the page's own query
    void goto((to === 'forgot' ? resolve('/login/forgot') : resolve('/login')) + query, { reset: false });
  }
</script>

<LoginPage {route} {navigate} home={() => void goto(resolve('/'))} />
{@render children()}
