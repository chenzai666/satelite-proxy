import { invoke } from "@tauri-apps/api/core";
import type { UiMode } from "./UiModeContext";

/** Pro console — matches tauri.conf.json default. */
export const PRO_WINDOW = { width: 960, height: 720 } as const;
/** Simple vertical strip — content ~380–400px + chrome. */
export const SIMPLE_WINDOW = { width: 420, height: 720 } as const;

/** Persist mode for next WebView recreate (Rust reads app_data/data/ui_mode). */
export async function persistUiModePref(mode: UiMode): Promise<void> {
  try {
    await invoke("set_ui_mode_pref", { mode });
  } catch {
    /* browser / missing command */
  }
}

/** Resize main window for the active UI mode (no-op outside Tauri). */
export async function applyWindowSizeForUiMode(mode: UiMode): Promise<void> {
  const size = mode === "simple" ? SIMPLE_WINDOW : PRO_WINDOW;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const { LogicalSize } = await import("@tauri-apps/api/dpi");
    const win = getCurrentWindow();
    await win.setSize(new LogicalSize(size.width, size.height));
    try {
      await win.setResizable(false);
    } catch {
      /* optional */
    }
  } catch {
    /* browser / missing permission */
  }
}
