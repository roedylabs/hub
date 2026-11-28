import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { svelteTesting } from '@testing-library/svelte/vite';
import { defineConfig } from 'vitest/config';

// Like a Roedy app: a SvelteKit SPA with the static adapter. The test is `*.spec.ts`, so that the root's
// `node --test scripts/` leaves it alone.
export default defineConfig({
  plugins: [sveltekit({ adapter: adapter({ fallback: 'index.html' }) }), svelteTesting()],
  test: { environment: 'jsdom', include: ['src/**/*.spec.ts'], setupFiles: ['./vitest-setup.ts'] },
});
