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
import type { HeroStyle, ThemeId } from "../types";
import { applyAccentToDom, resolveAccent } from "./accents";

const THEME_KEY = "satelite.theme";
const ACCENT_KEY = "satelite.accent";

export function normalizeTheme(raw: string | null | undefined): ThemeId {
  const t = (raw ?? "").trim().toLowerCase();
  if (t === "aerospace") return "aerospace";
  return "day";
}

export function readStoredTheme(): ThemeId {
  try {
    return normalizeTheme(localStorage.getItem(THEME_KEY));
  } catch {
    return "day";
  }
}

export function readStoredAccent(): string {
  try {
    return resolveAccent(localStorage.getItem(ACCENT_KEY)).id;
  } catch {
    return "green";
  }
}

function persistThemePref(theme: ThemeId, accent: string) {
  try {
    localStorage.setItem(THEME_KEY, theme);
    localStorage.setItem(ACCENT_KEY, accent);
  } catch {
    /* ignore */
  }
}

export function normalizeHeroStyle(raw: string | null | undefined): HeroStyle {
  const t = (raw ?? "").trim().toLowerCase();
  if (t === "classic") return "classic";
  if (t === "smiley") return "smiley";
  return "particle";
}

export function applyThemeToDom(theme: ThemeId, accent: string) {
  document.documentElement.dataset.theme = theme;
  // Drive native <select> / form control chrome (WKWebView) with the UI theme.
  document.documentElement.style.colorScheme =
    theme === "day" ? "light" : "dark";
  applyAccentToDom(accent, theme);
  persistThemePref(theme, accent);
}

/** Toggle the frosted-glass control look (see docs/webview2-memory-optimization-plan.md). */
export function applyGlassFrostToDom(frost: boolean) {
  if (frost) document.documentElement.dataset.glassFrost = "true";
  else delete document.documentElement.dataset.glassFrost;
}

interface ThemeContextValue {
  theme: ThemeId;
  setTheme: (next: ThemeId) => Promise<void>;
  accent: string;
  setAccent: (next: string) => Promise<void>;
  heroStyle: HeroStyle;
  setHeroStyle: (next: HeroStyle) => Promise<void>;
  glassFrost: boolean;
  setGlassFrost: (next: boolean) => Promise<void>;
  ready: boolean;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<ThemeId>(readStoredTheme);
  const [accent, setAccentState] = useState<string>(readStoredAccent);
  const [heroStyle, setHeroStyleState] = useState<HeroStyle>("particle");
  const [glassFrost, setGlassFrostState] = useState(false);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void getSettings()
      .then((s) => {
        if (cancelled) return;
        const nextTheme = normalizeTheme(s.theme);
        const nextAccent = resolveAccent(s.accent).id;
        const nextHero = normalizeHeroStyle(s.hero_style);
        setThemeState(nextTheme);
        setAccentState(nextAccent);
        setHeroStyleState(nextHero);
        setGlassFrostState(s.glass_frost === true);
        applyThemeToDom(nextTheme, nextAccent);
        applyGlassFrostToDom(s.glass_frost === true);
      })
      .catch(() => {
        applyThemeToDom(readStoredTheme(), readStoredAccent());
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

  const setHeroStyle = useCallback(async (next: HeroStyle) => {
    const style = normalizeHeroStyle(next);
    setHeroStyleState(style);
    try {
      await updateSettings({ heroStyle: style });
    } catch {
      /* UI already switched */
    }
  }, []);

  const setGlassFrost = useCallback(async (next: boolean) => {
    setGlassFrostState(next);
    applyGlassFrostToDom(next);
    try {
      await updateSettings({ glassFrost: next });
    } catch {
      /* UI already switched */
    }
  }, []);

  const value = useMemo(
    () => ({
      theme,
      setTheme,
      accent,
      setAccent,
      heroStyle,
      setHeroStyle,
      glassFrost,
      setGlassFrost,
      ready,
    }),
    [
      theme,
      setTheme,
      accent,
      setAccent,
      heroStyle,
      setHeroStyle,
      glassFrost,
      setGlassFrost,
      ready,
    ],
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
