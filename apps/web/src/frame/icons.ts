/**
 * The few Tabler icons of the app switcher as path data (24 × 24, stroke 2),
 * taken from `@tabler/icons-svelte-runes` (MIT), which the web app already
 * uses: the switcher is plain DOM and cannot use the Svelte components.
 */
export const ICONS = {
  grid: [
    'M4 5a1 1 0 1 0 2 0a1 1 0 1 0 -2 0',
    'M11 5a1 1 0 1 0 2 0a1 1 0 1 0 -2 0',
    'M18 5a1 1 0 1 0 2 0a1 1 0 1 0 -2 0',
    'M4 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0',
    'M11 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0',
    'M18 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0',
    'M4 19a1 1 0 1 0 2 0a1 1 0 1 0 -2 0',
    'M11 19a1 1 0 1 0 2 0a1 1 0 1 0 -2 0',
    'M18 19a1 1 0 1 0 2 0a1 1 0 1 0 -2 0',
  ],
  home: [
    'M5 12l-2 0l9 -9l9 9l-2 0',
    'M5 12v7a2 2 0 0 0 2 2h10a2 2 0 0 0 2 -2v-7',
    'M9 21v-6a2 2 0 0 1 2 -2h2a2 2 0 0 1 2 2v6',
  ],
  account: [
    'M3 12a9 9 0 1 0 18 0a9 9 0 1 0 -18 0',
    'M9 10a3 3 0 1 0 6 0a3 3 0 1 0 -6 0',
    'M6.168 18.849a4 4 0 0 1 3.832 -2.849h4a4 4 0 0 1 3.834 2.855',
  ],
  admin: ['M12 3a12 12 0 0 0 8.5 3a12 12 0 0 1 -8.5 15a12 12 0 0 1 -8.5 -15a12 12 0 0 0 8.5 -3'],
  retry: ['M20 11a8.1 8.1 0 0 0 -15.5 -2m-.5 -4v4h4', 'M4 13a8.1 8.1 0 0 0 15.5 2m.5 4v-4h-4'],
  signIn: [
    'M9 8v-2a2 2 0 0 1 2 -2h7a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-7a2 2 0 0 1 -2 -2v-2',
    'M3 12h13l-3 -3',
    'M13 15l3 -3',
  ],
  offline: [
    'M20 16l-4 4',
    'M7 12l5 5l-1.5 1.5a3.536 3.536 0 1 1 -5 -5l1.5 -1.5',
    'M17 12l-5 -5l1.5 -1.5a3.536 3.536 0 1 1 5 5l-1.5 1.5',
    'M3 21l2.5 -2.5',
    'M18.5 5.5l2.5 -2.5',
    'M10 11l-2 2',
    'M13 14l-2 2',
    'M16 16l4 4',
  ],
} as const;

export type IconName = keyof typeof ICONS;

const SVG = 'http://www.w3.org/2000/svg';

/** A decorative icon (`aria-hidden`), sized by the font size like the web app's. */
export function icon(doc: Document, name: IconName): SVGSVGElement {
  const svg = doc.createElementNS(SVG, 'svg');
  for (const [key, value] of [
    ['viewBox', '0 0 24 24'],
    ['fill', 'none'],
    ['stroke', 'currentColor'],
    ['stroke-width', '2'],
    ['stroke-linecap', 'round'],
    ['stroke-linejoin', 'round'],
    ['aria-hidden', 'true'],
    ['focusable', 'false'],
    ['class', 'icon'],
  ] as const) {
    svg.setAttribute(key, value);
  }
  for (const d of ICONS[name]) {
    const path = doc.createElementNS(SVG, 'path');
    path.setAttribute('d', d);
    svg.append(path);
  }
  return svg;
}
