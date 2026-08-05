import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { applyWindowSizeForUiMode, persistUiModePref } from "./windowLayout";

export type UiMode = "pro" | "simple";

const STORAGE_KEY = "satelite.uiMode";

function readStored(): UiMode {
  try {
    const v = localStorage.getItem(STORAGE_KEY)?.trim().toLowerCase();
    if (v === "simple" || v === "pro") return v;
  } catch {
    /* ignore */
  }
  return "pro";
}

interface UiModeContextValue {
  mode: UiMode;
  /** True after first window-size sync — avoid painting wrong shell. */
  layoutReady: boolean;
  setMode: (mode: UiMode) => void;
  toggleMode: () => void;
}

const UiModeContext = createContext<UiModeContextValue | null>(null);

export function UiModeProvider({ children }: { children: ReactNode }) {
  const [mode, setModeState] = useState<UiMode>(() => readStored());
  const [layoutReady, setLayoutReady] = useState(false);

  // On mount: persist + size before showing shell (prevents pro flash on wake).
  useEffect(() => {
    let cancelled = false;
    const initial = readStored();
    setModeState(initial);
    void (async () => {
      await persistUiModePref(initial);
      await applyWindowSizeForUiMode(initial);
      if (!cancelled) setLayoutReady(true);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const setMode = useCallback((next: UiMode) => {
    try {
      localStorage.setItem(STORAGE_KEY, next);
      document.documentElement.dataset.uiMode = next;
    } catch {
      /* ignore */
    }
    // Resize first, then swap shell — avoids full-width simple / narrow pro frames.
    void (async () => {
      await persistUiModePref(next);
      await applyWindowSizeForUiMode(next);
      setModeState(next);
    })();
  }, []);

  const toggleMode = useCallback(() => {
    setMode(mode === "pro" ? "simple" : "pro");
  }, [mode, setMode]);

  const value = useMemo(
    () => ({ mode, layoutReady, setMode, toggleMode }),
    [mode, layoutReady, setMode, toggleMode],
  );

  return (
    <UiModeContext.Provider value={value}>{children}</UiModeContext.Provider>
  );
}

export function useUiMode(): UiModeContextValue {
  const ctx = useContext(UiModeContext);
  if (!ctx) {
    throw new Error("useUiMode must be used within UiModeProvider");
  }
  return ctx;
}
