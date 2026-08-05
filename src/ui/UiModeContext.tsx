import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { applyWindowSizeForUiMode } from "./windowLayout";

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
  setMode: (mode: UiMode) => void;
  toggleMode: () => void;
}

const UiModeContext = createContext<UiModeContextValue | null>(null);

export function UiModeProvider({ children }: { children: ReactNode }) {
  const [mode, setModeState] = useState<UiMode>(() => readStored());

  const setMode = useCallback((next: UiMode) => {
    setModeState(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      /* ignore */
    }
    void applyWindowSizeForUiMode(next);
  }, []);

  // Apply size on first paint (e.g. last session was simple).
  useEffect(() => {
    void applyWindowSizeForUiMode(mode);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only once on mount for stored mode
  }, []);

  const toggleMode = useCallback(() => {
    setMode(mode === "pro" ? "simple" : "pro");
  }, [mode, setMode]);

  const value = useMemo(
    () => ({ mode, setMode, toggleMode }),
    [mode, setMode, toggleMode],
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
