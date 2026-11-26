/**
 * Requests to one resource one after another (e.g. the saves of `admin/org` across page instances): each task
 * starts once the previous one has settled, also if it failed; without a running one it starts at once. The server
 * then commits the writes of this browser in the order they started, and the responses arrive in that order, so
 * the last one applied is the last one stored. Writes from another browser are not covered.
 */
export function serial(): <T>(task: () => Promise<T>) => Promise<T> {
  let tail: Promise<void> | null = null;
  return <T>(task: () => Promise<T>): Promise<T> => {
    const run = tail ? tail.then(task) : task();
    const done = run.then(
      () => {},
      () => {},
    );
    tail = done;
    void done.then(() => {
      if (tail === done) tail = null;
    });
    return run;
  };
}
