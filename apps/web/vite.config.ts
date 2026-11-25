import { paraglideVitePlugin } from '@inlang/paraglide-js';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { svelteTesting } from '@testing-library/svelte/vite';
import { defineConfig } from 'vitest/config';

// Backend for `pnpm dev` (cargo run -- serve); /api and the health checks go there.
const backend = process.env.ROEDY_DEV_BACKEND || 'http://127.0.0.1:8080';

export default defineConfig({
  plugins: [
    paraglideVitePlugin({
      project: './project.inlang',
      outdir: './src/lib/paraglide',
      // Same as in the `sync` script (package.json).
      strategy: ['baseLocale'],
    }),
    sveltekit({
      // SPA: one index.html for all routes, which the backend serves for unknown paths.
      adapter: adapter({ fallback: 'index.html' }),
      // The hub's assets live under /_hub/, a path no app can take as its slug (docs/app-contract.md › Routing).
      appDir: '_hub',
    }),
  ],
  server: {
    proxy: {
      '/api': backend,
      '/healthz': backend,
      '/readyz': backend,
    },
  },
  test: {
    projects: [
      {
        extends: true,
        plugins: [svelteTesting()],
        test: {
          name: 'web',
          environment: 'jsdom',
          include: ['src/**/*.test.ts'],
          setupFiles: ['./vitest-setup.ts'],
        },
      },
    ],
  },
});
