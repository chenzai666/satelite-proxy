import { invoke } from "@tauri-apps/api/core";
import type { UiMode } from "./UiModeContext";

/** Pro console — matches tauri.conf.json default. */
export const PRO_WINDOW = { width: 960, height: 720 } as const;
/** Simple vertical strip — content ~380–400px + chrome. */
export const SIMPLE_WINDOW = { width: 420, height: 720 } as const;
/** Simple mode is user-resizable; content scrolls below this floor. */
export const SIMPLE_MIN = { width: 320, height: 480 } as const;
/** …and can only shrink — never grow past the default simple strip. */
export const SIMPLE_MAX = SIMPLE_WINDOW;

const SIZE_KEY = "satelite.simpleWindowSize";

/** Persist mode for next WebView recreate (Rust reads app_data/data/ui_mode). */
export async function persistUiModePref(mode: UiMode): Promise<void> {
  try {
    await invoke("set_ui_mode_pref", { mode });
  } catch {
    /* browser / missing command */
  }
}

function clampSimpleSize(width: number, height: number) {
  return {
    width: Math.min(Math.max(Math.round(width), SIMPLE_MIN.width), SIMPLE_MAX.width),
    height: Math.min(Math.max(Math.round(height), SIMPLE_MIN.height), SIMPLE_MAX.height),
  };
}

function readSimpleWindowSize(): { width: number; height: number } | null {
  try {
    const raw = localStorage.getItem(SIZE_KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as { width?: unknown; height?: unknown };
    if (typeof v.width !== "number" || typeof v.height !== "number") return null;
    if (!Number.isFinite(v.width) || !Number.isFinite(v.height)) return null;
    return clampSimpleSize(v.width, v.height);
  } catch {
    return null;
  }
}

/**
 * Save the simple-mode window size (debounced) so it survives WebView
 * recreate and app restarts. Restore happens in applyWindowSizeForUiMode.
 */
export function watchSimpleWindowSize(): () => void {
  let timer: number | undefined;
  const onResize = () => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      try {
        const size = clampSimpleSize(window.innerWidth, window.innerHeight);
        localStorage.setItem(SIZE_KEY, JSON.stringify(size));
      } catch {
        /* ignore */
      }
    }, 300);
  };
  window.addEventListener("resize", onResize);
  return () => {
    window.removeEventListener("resize", onResize);
    window.clearTimeout(timer);
  };
}

/** Apply window size / resize policy for the active UI mode (no-op outside Tauri). */
export async function applyWindowSizeForUiMode(mode: UiMode): Promise<void> {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const { LogicalSize } = await import("@tauri-apps/api/dpi");
    const win = getCurrentWindow();
    if (mode === "simple") {
      // Allow the user to shrink the strip; keep the saved size if any.
      await win.setMinSize(new LogicalSize(SIMPLE_MIN.width, SIMPLE_MIN.height));
      await win.setMaxSize(new LogicalSize(SIMPLE_MAX.width, SIMPLE_MAX.height));
      await win.setResizable(true);
      const size = readSimpleWindowSize() ?? SIMPLE_WINDOW;
      await win.setSize(new LogicalSize(size.width, size.height));
    } else {
      await win.setMinSize(null);
      await win.setMaxSize(null);
      await win.setSize(new LogicalSize(PRO_WINDOW.width, PRO_WINDOW.height));
      try {
        await win.setResizable(false);
      } catch {
        /* optional */
      }
    }
  } catch {
    /* browser / missing permission */
  }
}
