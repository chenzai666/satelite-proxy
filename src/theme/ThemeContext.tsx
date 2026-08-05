import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { getSettings, updateSettings } from "../api";
import type { ThemeId } from "../types";

export function normalizeTheme(raw: string | null | undefined): ThemeId {
  const t = (raw ?? "").trim().toLowerCase();
  if (t === "aerospace") return "aerospace";
  return "day";
}

export function applyThemeToDom(theme: ThemeId) {
  document.documentElement.dataset.theme = theme;
  // Drive native <select> / form control chrome (WKWebView) with the UI theme.
  document.documentElement.style.colorScheme =
    theme === "day" ? "light" : "dark";
}

interface ThemeContextValue {
  theme: ThemeId;
  setTheme: (next: ThemeId) => Promise<void>;
  ready: boolean;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<ThemeId>("day");
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void getSettings()
      .then((s) => {
        if (cancelled) return;
        const next = normalizeTheme(s.theme);
        setThemeState(next);
        applyThemeToDom(next);
      })
      .catch(() => {
        applyThemeToDom("day");
      })
      .finally(() => {
        if (!cancelled) setReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setTheme = useCallback(async (next: ThemeId) => {
    setThemeState(next);
    applyThemeToDom(next);
    try {
      await updateSettings({ theme: next });
    } catch {
      /* UI already switched */
    }
  }, []);

  const value = useMemo(
    () => ({ theme, setTheme, ready }),
    [theme, setTheme, ready],
  );

  return (
    <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) {
    throw new Error("useTheme must be used within ThemeProvider");
  }
  return ctx;
}
