/**
 * Shared bookkeeping for the root-zoom viewport scale (useViewportScale).
 *
 * While the `zoom` transition on <html> is animating (maximize / restore),
 * DOM measurements are garbage: clientWidth/scrollWidth/computed font-size
 * read intermediate states, so fit-style routines (see useSingleLineFit in
 * DashboardPage) would write bogus inline sizes that persist after the
 * animation ends.
 *
 * Contract:
 * - `useViewportScale` calls `markZoomChanged()` whenever it rewrites zoom.
 * - Measurement-driven code skips work while `isZoomSettling()` is true and
 *   re-runs on the synthetic `resize` dispatched once the transition ends.
 */

/** CSS transition on html { zoom } is 180ms — settle after it plus margin. */
const SETTLE_MS = 240;

let lastChangeAt = 0;
let settleTimer: number | undefined;

/** Record a zoom rewrite and schedule the at-rest refit dispatch. */
export function markZoomChanged(): void {
  lastChangeAt = Date.now();
  window.clearTimeout(settleTimer);
  settleTimer = window.setTimeout(() => {
    window.dispatchEvent(new Event("resize"));
  }, SETTLE_MS);
}

/** True while the zoom transition is still animating — skip measuring. */
export function isZoomSettling(): boolean {
  return Date.now() - lastChangeAt < SETTLE_MS;
}
