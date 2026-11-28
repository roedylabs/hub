/**
 * Targets of Account › Accounts for its routes (`/account/accounts`, `…/new`, `…/[id]`): the pages themselves get
 * them as callbacks, so that their tests need no router.
 */
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import type { AccountsNav } from '#lib/accounts.svelte.ts';
import { leaveTo } from '#lib/navigate.ts';
import type { AccountNav } from '#lib/stalwart-account.svelte.ts';

/** Of the list and the form; `next` goes along to the form. */
export function accountsNav(next: string | null): AccountsNav {
  return {
    add: () => {
      const to = resolve('/account/accounts/new');
      // eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve() plus the checked `next`
      void goto(next ? `${to}?${new URLSearchParams({ next }).toString()}` : to);
    },
    list: () => void goto(resolve('/account/accounts')),
    account: (id) => void goto(resolve('/account/accounts/[id]', { id })),
    servers: () => void goto(resolve('/admin/stalwart')),
    leave: (to) => leaveTo(to),
  };
}

/** Of an account's page. */
export function accountNav(): AccountNav {
  return {
    list: () => void goto(resolve('/account/accounts')),
    servers: () => void goto(resolve('/admin/stalwart')),
  };
}
