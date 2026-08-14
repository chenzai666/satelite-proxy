import { useEffect, useRef } from "react";

/**
 * setInterval that only runs while the document is visible.
 * Pauses when minimized / backgrounded / tab hidden → saves CPU & GC churn.
 */
export function useVisibleInterval(
  callback: () => void | Promise<unknown>,
  delayMs: number | null,
  /** Also fire immediately when becoming visible (default true). */
  runOnVisible = true,
) {
  const cbRef = useRef(callback);
  cbRef.current = callback;

  useEffect(() => {
    if (delayMs == null || delayMs <= 0) return;

    let id: number | null = null;
    let inFlight = false;

    const run = () => {
      if (inFlight) return;

      let result: void | Promise<unknown>;
      try {
        result = cbRef.current();
      } catch (error) {
        console.error("visible interval callback failed", error);
        return;
      }

      if (result && typeof result.then === "function") {
        inFlight = true;
        void Promise.resolve(result)
          .catch((error) => {
            console.error("visible interval callback failed", error);
          })
          .finally(() => {
            inFlight = false;
          });
      }
    };

    const clear = () => {
      if (id != null) {
        window.clearInterval(id);
        id = null;
      }
    };

    const start = () => {
      clear();
      id = window.setInterval(() => {
        run();
      }, delayMs);
    };

    const sync = () => {
      if (document.visibilityState === "visible") {
        if (runOnVisible) run();
        start();
      } else {
        clear();
      }
    };

    sync();
    document.addEventListener("visibilitychange", sync);
    return () => {
      document.removeEventListener("visibilitychange", sync);
      clear();
    };
  }, [delayMs, runOnVisible]);
}
