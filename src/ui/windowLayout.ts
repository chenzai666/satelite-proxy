import type { UiMode } from "./UiModeContext";

/** Pro console — matches tauri.conf.json default. */
export const PRO_WINDOW = { width: 1024, height: 760 } as const;
/** Simple vertical strip — content ~380–400px + chrome. */
export const SIMPLE_WINDOW = { width: 420, height: 760 } as const;

/** Resize main window for the active UI mode (no-op outside Tauri). */
export async function applyWindowSizeForUiMode(mode: UiMode): Promise<void> {
  const size = mode === "simple" ? SIMPLE_WINDOW : PRO_WINDOW;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const { LogicalSize } = await import("@tauri-apps/api/dpi");
    const win = getCurrentWindow();
    await win.setSize(new LogicalSize(size.width, size.height));
    // Keep non-resizable; still enforce size after mode switch.
    try {
      await win.setResizable(false);
    } catch {
      /* optional on some platforms */
    }
  } catch {
    /* browser / missing permission */
  }
}
