/**
 * Style sheet of the app switcher's shadow root, built from the design tokens
 * (`@roedy/tokens`), so that it looks like the hub in every app. It is
 * adopted as a constructed style sheet, which a strict CSP (no `style-src`
 * at all) does not block.
 *
 * Custom properties reach into the shadow DOM: an app that sets the tokens'
 * properties on its page (`--pop`, `--fg`, `--acc`, `--font-sans` …, from
 * `@roedy/tokens`) gets its colours, accent, mode and fonts in the switcher as
 * well. Where a property is not set, the switcher falls back to the tokens'
 * values for light or dark (`.dark`). It uses them through private names
 * (`--_pop` …), so that the fallbacks never change the page's own.
 */
import {
  ACCENTS,
  DEFAULT_ACCENT,
  FONT_SANS,
  FONT_SERIF,
  KEYFRAMES,
  MODES,
  derivedAccentVars,
  type ModeName,
} from '@roedy/tokens';

/** The tokens the switcher uses; the others stay out of the script. */
const USED = [
  '--surface',
  '--surface2',
  '--sunken',
  '--pop',
  '--fg',
  '--mfg',
  '--hov',
  '--danger',
  '--sh1',
  '--sh3',
] as const;

/** Private name of a token: `--pop` → `--_pop`. */
const own = (name: string) => `--_${name.slice(2)}`;

/** The page's token if it sets one, else the tokens' value for `mode`. */
function modeVars(mode: ModeName): string {
  const vars: [string, string][] = USED.map((name) => [name, MODES[mode][name]]);
  for (const [name, value] of Object.entries(derivedAccentVars(mode))) {
    if (name === '--toastacc') continue;
    // The mixes refer to the private accent and surface.
    vars.push([name, value.replaceAll('var(--acc)', 'var(--_acc)').replaceAll('var(--surface)', 'var(--_surface)')]);
  }
  return vars.map(([name, value]) => `${own(name)}:var(${name},${value});`).join('');
}

export const CSS = `
:host{display:inline-block;vertical-align:middle;line-height:normal}
:host([hidden]){display:none}
.root{${modeVars('light')}--_acc:var(--acc,${ACCENTS[DEFAULT_ACCENT].base});--_dur:var(--dur,220ms);--_serif:var(--font-serif,${FONT_SERIF});--_sans:var(--font-sans,${FONT_SANS});color-scheme:light;color:var(--_fg);font-family:var(--_sans);font-size:16px;-webkit-font-smoothing:antialiased}
.root.dark{${modeVars('dark')}color-scheme:dark}
*,*::before,*::after{box-sizing:border-box}
button,a{font:inherit;color:inherit;text-decoration:none;background:none;border:0;margin:0;padding:0;cursor:pointer;-webkit-tap-highlight-color:transparent}
.icon{width:1em;height:1em;flex-shrink:0;display:block}
:focus{outline:none}
:focus-visible{outline:2px solid var(--_acc);outline-offset:2px}

.toggle{width:40px;height:40px;border-radius:9999px;display:grid;place-items:center;color:var(--_mfg);font-size:20px;transition:background-color var(--_dur),color var(--_dur)}
.toggle[aria-expanded="true"]{background:var(--_accsoft);color:var(--_deep)}
@media (hover:hover){.toggle:hover{background:var(--_hov);color:var(--_fg)}}

.pop{position:fixed;inset:auto;margin:0;border:0;padding:18px;width:min(300px,calc(100vw - 24px));max-height:calc(100vh - 24px);overflow:auto;overscroll-behavior:contain;border-radius:28px;background:var(--_pop);color:var(--_fg);box-shadow:var(--_sh3);cursor:default;animation:snPop 260ms cubic-bezier(.2,1.2,.3,1)}
.pop[hidden]{display:none}
@keyframes snPop{${KEYFRAMES.snPop}}
@media (prefers-reduced-motion:reduce){.pop{animation:none}.toggle{transition:none}}

.title{font-family:var(--_serif);font-size:18px;line-height:1.3;padding:0 4px 14px}
.msg{margin:0;padding:6px 4px 16px;font-size:14px;line-height:1.45;color:var(--_mfg)}
.apps{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:8px}
.sep{height:1px;margin:12px 4px;background:var(--_sunken)}

.app,.row{outline-offset:-2px}
.app{position:relative;min-width:0;display:flex;flex-direction:column;align-items:center;gap:8px;border-radius:20px;padding:14px 4px 12px;transition:background-color var(--_dur)}
.app[aria-current="page"]{background:var(--_surface2)}
.app:hover{background:var(--_hov)}
.app[aria-current="page"]:hover{background:var(--_sunken)}
.tile{width:44px;height:44px;flex-shrink:0;border-radius:15px;display:grid;place-items:center;background:var(--_surface);box-shadow:var(--_sh1)}
.tile img{display:block;width:66%;height:66%}
.offline .tile img{opacity:.5}
.text{min-width:0;max-width:100%;display:flex;flex-direction:column;align-items:center;gap:2px}
.name{max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12.5px;font-weight:700}
.badge{display:inline-flex;align-items:center;gap:4px;font-size:11px;font-weight:800;color:var(--_danger)}

.row{width:100%;height:44px;display:flex;align-items:center;gap:14px;padding:0 14px;border-radius:16px;font-size:14.5px;font-weight:600;text-align:left;transition:background-color var(--_dur)}
.row:hover{background:var(--_hov)}
.row .icon{font-size:20px;color:var(--_mfg)}
`;
