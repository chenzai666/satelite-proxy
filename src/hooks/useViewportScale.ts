import { useEffect } from "react";
import type { UiMode } from "../ui/UiModeContext";
import { PRO_WINDOW, SIMPLE_WINDOW } from "../ui/windowLayout";
import { markZoomChanged } from "./viewportScale";

/**
 * Magnify the whole UI when the OS window grows past the design size
 * (Windows maximize etc.). Sets CSS `zoom` on the root element, so the
 * app renders at design proportions scaled up — instead of a small UI
 * floating in a maximized window. Sets `data-ui-scaled` on <html> for
 * companion CSS (centering the simple strip).
 *
 * Scale = min(width / designWidth, height / designHeight) — the full
 * design area stays visible; the extra width is used by the fluid layout.
 *
 * Window size comes from the Tauri window API (OS-level px), never
 * `window.innerWidth`: root zoom changes what innerWidth reports, so a
 * DOM measurement would feed back into the scale and oscillate.
 * `window.devicePixelRatio` is the OS DPI ratio — element zoom cannot
 * change it, so physical→logical conversion stays stable.
 */
export function useViewportScale(mode: UiMode): void {
  useEffect(() => {
    const design = mode === "simple" ? SIMPLE_WINDOW : PRO_WINDOW;
    let disposed = false;
    let unlisten: (() => void) | undefined;

    const applyScale = (logicalWidth: number, logicalHeight: number) => {
      const fit = Math.min(
        logicalWidth / design.width,
        logicalHeight / design.height,
      );
      // Scale up only: windowed sizes stay pixel-exact (no zoom).
      // Quantize to 1% so drag-resize writes a stable style value.
      const scale = fit > 1.02 ? Math.round(fit * 100) / 100 : 1;
      const root = document.documentElement;
      const nextZoom = scale === 1 ? "" : String(scale);
      // Skip no-op rewrites so repeated resize events don't retrigger the
      // transition / settle dispatch.
      if (root.style.zoom === nextZoom) return;
      root.style.zoom = nextZoom;
      if (scale === 1) {
        root.removeAttribute("data-ui-scaled");
      } else {
        root.setAttribute("data-ui-scaled", "1");
      }
      // Measurement-driven code skips while the transition animates and
      // refits on the at-rest resize this schedules (see viewportScale.ts).
      markZoomChanged();
    };

    const applyPhysical = (width: number, height: number) => {
      const dpr = window.devicePixelRatio || 1;
      applyScale(width / dpr, height / dpr);
    };

    // Initial size (permission already granted in capabilities).
    const measure = async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const size = await getCurrentWindow().innerSize();
        if (disposed) return;
        applyPhysical(size.width, size.height);
      } catch {
        /* plain browser dev: keep zoom 1 */
      }
    };

    // Scale changes only on one-shot transitions (maximize / restore /
    // mode switch) — the pro window is not user-resizable and simple mode
    // is capped at its design size — so no debounce is needed.
    void measure();
    void import("@tauri-apps/api/window")
      .then(({ getCurrentWindow }) =>
        getCurrentWindow().onResized((event) =>
          applyPhysical(event.payload.width, event.payload.height),
        ),
      )
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      })
      .catch(() => {
        /* plain browser dev */
      });

    return () => {
      disposed = true;
      unlisten?.();
      document.documentElement.style.zoom = "";
      document.documentElement.removeAttribute("data-ui-scaled");
    };
  }, [mode]);
}
