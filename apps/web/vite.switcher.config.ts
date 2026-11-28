import { defineConfig } from 'vite';

/**
 * Build of the app switcher (`src/switcher/`) into one dependency-free ES
 * module, `build/_hub/app-switcher.js`, after the SvelteKit build (`build`
 * script): the hub serves it as `GET /api/v1/app-switcher.js`
 * (crates/server/src/web/switcher.rs). Not hashed, since apps load it by a
 * fixed address; the hub has the browser revalidate it via its ETag.
 */
export default defineConfig({
  publicDir: false,
  build: {
    outDir: 'build/_hub',
    emptyOutDir: false,
    target: 'es2022',
    sourcemap: false,
    minify: true,
    lib: {
      entry: 'src/switcher/app-switcher.ts',
      formats: ['es'],
      fileName: () => 'app-switcher.js',
    },
    // Library mode keeps the whitespace of ES modules; the switcher is a finished file, not a library.
    rolldownOptions: { output: { minify: true } },
  },
});
