/**
 * Sections of a page in the account and administration area (`data-sec`) and
 * the tabs above them (e.g. Settings › Appearance): scrolls the content so
 * that the section starts 24 px below its top edge.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/sections.ts`) without
 * marking a section opened from the search.
 */

/** Scroll to the section `[data-sec="<key>"]` in `root`; instantly without animations. */
export function scrollToSection(root: HTMLElement | undefined, key: string, animate: boolean): void {
  const section = root?.querySelector<HTMLElement>(`[data-sec="${key}"]`);
  // The scrolling content (card or mobile panel) is positioned and therefore the `offsetParent`.
  const container = section?.offsetParent;
  if (!section || !(container instanceof HTMLElement)) return;
  container.scrollTo({ top: Math.max(0, section.offsetTop - 24), behavior: animate ? 'smooth' : 'auto' });
}
