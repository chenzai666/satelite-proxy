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
import {
  normalizeLocale,
  translate,
  type Locale,
  type MessageKey,
} from "./messages";

type TFn = (key: MessageKey, vars?: Record<string, string | number>) => string;

interface LocaleContextValue {
  locale: Locale;
  setLocale: (next: Locale) => Promise<void>;
  t: TFn;
  ready: boolean;
}

const LocaleContext = createContext<LocaleContextValue | null>(null);

export function LocaleProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>("zh");
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void getSettings()
      .then((s) => {
        if (!cancelled) setLocaleState(normalizeLocale(s.locale));
      })
      .catch(() => {
        /* keep default */
      })
      .finally(() => {
        if (!cancelled) setReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setLocale = useCallback(async (next: Locale) => {
    setLocaleState(next);
    try {
      await updateSettings({ locale: next });
    } catch {
      /* UI already switched; next save may fix */
    }
  }, []);

  const t = useCallback<TFn>(
    (key, vars) => translate(locale, key, vars),
    [locale],
  );

  const value = useMemo(
    () => ({ locale, setLocale, t, ready }),
    [locale, setLocale, t, ready],
  );

  return (
    <LocaleContext.Provider value={value}>{children}</LocaleContext.Provider>
  );
}

export function useI18n(): LocaleContextValue {
  const ctx = useContext(LocaleContext);
  if (!ctx) {
    throw new Error("useI18n must be used within LocaleProvider");
  }
  return ctx;
}
