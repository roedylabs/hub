/**
 * Dragging the dialogs of the desktop layout down to close them
 * (`dragSheet` in `swipe.ts`): the dialog lies inside its backdrop.
 *
 * Taken over from Roedy Mail (`apps/web/src/lib/dialog-drag.ts`).
 */
import type { Attachment } from 'svelte/attachments';
import { ui } from './state/ui.svelte.ts';
import { dragSheet } from './swipe.ts';

/**
 * Only on a tablet (coarse primary pointer without the mobile layout,
 * `ui.swipeOn`) and as a sheet on mobile, not with a mouse on the desktop;
 * `can` turns it off for a layer that its backdrop does not close either.
 */
export const dialogDrag = (can: () => boolean = () => true): Attachment<HTMLElement> =>
  dragSheet(() => (ui.swipeOn && can() ? { animations: ui.motion } : null));
