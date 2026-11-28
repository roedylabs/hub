/**
 * The hub's frame: the web components that every app shows the same way,
 * served by the hub at run time as one dependency-free ES module,
 * `GET /api/v1/frame.js` (docs/app-contract.md › The hub's frame, docs/plan.md ›
 * Shared interface and packages). Importing it defines the elements; for now
 * `<roedy-app-switcher>`, later the notification bell and the notice bar.
 */
export { RoedyAppSwitcher } from './app-switcher.ts';
