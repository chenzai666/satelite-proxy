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
import { applyAccentToDom, resolveAccent } from "./accents";

export function normalizeTheme(raw: string | null | undefined): ThemeId {
  const t = (raw ?? "").trim().toLowerCase();
  if (t === "aerospace") return "aerospace";
  return "day";
}

export function applyThemeToDom(theme: ThemeId, accent: string) {
  document.documentElement.dataset.theme = theme;
  // Drive native <select> / form control chrome (WKWebView) with the UI theme.
  document.documentElement.style.colorScheme =
    theme === "day" ? "light" : "dark";
  applyAccentToDom(accent, theme);
}

interface ThemeContextValue {
  theme: ThemeId;
  setTheme: (next: ThemeId) => Promise<void>;
  accent: string;
  setAccent: (next: string) => Promise<void>;
  ready: boolean;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<ThemeId>("day");
  const [accent, setAccentState] = useState<string>("green");
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void getSettings()
      .then((s) => {
        if (cancelled) return;
        const nextTheme = normalizeTheme(s.theme);
        const nextAccent = resolveAccent(s.accent).id;
        setThemeState(nextTheme);
        setAccentState(nextAccent);
        applyThemeToDom(nextTheme, nextAccent);
      })
      .catch(() => {
        applyThemeToDom("day", "green");
      })
      .finally(() => {
        if (!cancelled) setReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setTheme = useCallback(
    async (next: ThemeId) => {
      setThemeState(next);
      applyThemeToDom(next, accent);
      try {
        await updateSettings({ theme: next });
      } catch {
        /* UI already switched */
      }
    },
    [accent],
  );

  const setAccent = useCallback(
    async (next: string) => {
      const id = resolveAccent(next).id;
      setAccentState(id);
      applyThemeToDom(theme, id);
      try {
        await updateSettings({ accent: id });
      } catch {
        /* UI already switched */
      }
    },
    [theme],
  );

  const value = useMemo(
    () => ({ theme, setTheme, accent, setAccent, ready }),
    [theme, setTheme, accent, setAccent, ready],
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
