/**
 * Reactive stand-in for `page` from `$app/state` in tests: assigning `url`
 * updates a mounted layout like a navigation. Not a test file: imported by
 * `routes/layout.test.ts`.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/test/page.svelte.ts`).
 */
export const page = $state({
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- the tests replace `url` as a whole, like SvelteKit's `page`
  url: new URL('http://localhost/'),
  status: 200,
  params: {},
  error: null as { message: string } | null,
});
