import { useEffect, useState } from "react";

/**
 * Global "core is starting/stopping/restarting" flag.
 * TopNav (and simple shell status) spin while depth > 0.
 * Depth covers overlapping invoke calls + background apply events.
 *
 * Fast backend ops used to clear busy in <100ms (spinner flash). A minimum
 * hold keeps the ring animation readable without blocking longer restarts.
 */
const DEFAULT_MIN_MS = 550;

let depth = 0;
const listeners = new Set<(busy: boolean) => void>();

function emit() {
  const busy = depth > 0;
  for (const l of listeners) l(busy);
}

/**
 * Increment busy depth. The returned ender clears this token, holding the
 * spinner for at least `minMs` from begin so short restarts don't flash.
 */
export function beginCoreBusy(minMs = DEFAULT_MIN_MS): () => Promise<void> {
  let closed = false;
  const started = performance.now();
  depth += 1;
  emit();
  return () => {
    if (closed) return Promise.resolve();
    closed = true;
    const wait = Math.max(0, minMs - (performance.now() - started));
    return new Promise<void>((resolve) => {
      const finish = () => {
        depth = Math.max(0, depth - 1);
        emit();
        resolve();
      };
      if (wait > 0) {
        window.setTimeout(finish, wait);
      } else {
        finish();
      }
    });
  };
}

/** Wrap a core lifecycle promise so the navbar spinner runs for its duration. */
export function trackCoreBusy<T>(
  p: Promise<T>,
  minMs = DEFAULT_MIN_MS,
): Promise<T> {
  const end = beginCoreBusy(minMs);
  // finally waits on end()'s promise → callers keep local "restarting" state
  // until the min hold elapses too.
  return p.finally(() => end());
}

export function isCoreBusy(): boolean {
  return depth > 0;
}

export function subscribeCoreBusy(
  listener: (busy: boolean) => void,
): () => void {
  listeners.add(listener);
  listener(depth > 0);
  return () => {
    listeners.delete(listener);
  };
}

export function useCoreBusy(): boolean {
  const [busy, setBusy] = useState(depth > 0);
  useEffect(() => subscribeCoreBusy(setBusy), []);
  return busy;
}
