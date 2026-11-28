import { defineConfig } from 'vite';

/**
 * Build of the hub's frame (`src/frame/`, the web components every app
 * shows) into one dependency-free ES module, `build/_hub/frame.js`, after the
 * SvelteKit build (`build` script): the hub serves it as `GET
 * /api/v1/frame.js` (crates/server/src/web/frame.rs). Not hashed, since apps
 * load it by a fixed address; the hub has the browser revalidate it via its
 * ETag.
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
      entry: 'src/frame/frame.ts',
      formats: ['es'],
      fileName: () => 'frame.js',
    },
    // Library mode keeps the whitespace of ES modules; the frame is a finished file, not a library.
    rolldownOptions: { output: { minify: true } },
  },
});
