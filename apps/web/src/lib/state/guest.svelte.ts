/**
 * State module `guest`: what the guest pages (setup, sign-in, password reset,
 * invitation) share with the layout – the document title of the open screen
 * and the setup's hand-over to the home page. The rest of the shell's state
 * is `ui` of `@roedy/ui`.
 */
export class Guest {
  /**
   * Document title of the open guest page's screen ("Setup token · Setup"), set by its shell; `null` outside the
   * guest pages.
   */
  title = $state<string | null>(null);
  /** The setup has signed the administrator in and opens the home page: `/setup` stays until then. */
  setupLeaving = $state(false);

  /** Back to the start (sign-out). */
  reset(): void {
    this.title = null;
    this.setupLeaving = false;
  }
}

export const guest = new Guest();
