import { useEffect, useRef } from "react";

/**
 * setInterval that only runs while the document is visible.
 * Pauses when minimized / backgrounded / tab hidden → saves CPU & GC churn.
 */
export function useVisibleInterval(
  callback: () => void,
  delayMs: number | null,
  /** Also fire immediately when becoming visible (default true). */
  runOnVisible = true,
) {
  const cbRef = useRef(callback);
  cbRef.current = callback;

  useEffect(() => {
    if (delayMs == null || delayMs <= 0) return;

    let id: number | null = null;

    const clear = () => {
      if (id != null) {
        window.clearInterval(id);
        id = null;
      }
    };

    const start = () => {
      clear();
      id = window.setInterval(() => {
        cbRef.current();
      }, delayMs);
    };

    const sync = () => {
      if (document.visibilityState === "visible") {
        if (runOnVisible) cbRef.current();
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
