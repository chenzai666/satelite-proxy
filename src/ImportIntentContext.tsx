import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  parseImportDeepLinks,
  type ImportPrefill,
} from "./deepLink";

interface ImportIntentValue {
  /** Latest prefill from a one-click subscribe link (null after consume). */
  prefill: ImportPrefill | null;
  /** Bumps on every new deep link so pages re-open the modal. */
  token: number;
  /** Clear after the add form has applied the prefill (local React state only). */
  consume: () => void;
  /**
   * User closed or finished the add-subscription dialog.
   * Drops backend pending deep-link so tray/Dock wake will not reopen it.
   */
  dismiss: () => void;
}

const ImportIntentContext = createContext<ImportIntentValue | null>(null);

async function peekPendingUrls(): Promise<string[]> {
  try {
    const urls = await invoke<string[] | null>("peek_pending_import_urls");
    return urls ?? [];
  } catch {
    return [];
  }
}

async function clearPendingUrls(): Promise<void> {
  try {
    await invoke("clear_pending_import_urls");
  } catch {
    /* browser / missing command */
  }
}

export function ImportIntentProvider({ children }: { children: ReactNode }) {
  const [prefill, setPrefill] = useState<ImportPrefill | null>(null);
  const [token, setToken] = useState(0);
  /** Dedupe get-pending + delayed emit of the same open (~2.5s). */
  const lastLiveRef = useRef({ key: "", at: 0 });

  const applyUrls = useCallback((urls: string[]) => {
    const p = parseImportDeepLinks(urls);
    if (!p) return;
    const key = `${p.url}\0${p.name ?? ""}`;
    const now = Date.now();
    if (
      lastLiveRef.current.key === key &&
      now - lastLiveRef.current.at < 2500
    ) {
      return;
    }
    lastLiveRef.current = { key, at: now };
    setPrefill(p);
    setToken((n) => n + 1);
  }, []);

  const consume = useCallback(() => {
    setPrefill(null);
  }, []);

  const dismiss = useCallback(() => {
    setPrefill(null);
    void clearPendingUrls();
  }, []);

  useEffect(() => {
    let unlistenEvent: (() => void) | undefined;
    let cancelled = false;

    void (async () => {
      // Only while backend still has pending (cleared when user closes the modal).
      const pending = await peekPendingUrls();
      if (!cancelled && pending.length) applyUrls(pending);

      try {
        unlistenEvent = await listen<string[]>("deep-link-urls", (e) => {
          const urls = Array.isArray(e.payload) ? e.payload : [];
          applyUrls(urls);
        });
      } catch {
        /* not in Tauri */
      }
    })();

    return () => {
      cancelled = true;
      unlistenEvent?.();
    };
  }, [applyUrls]);

  const value = useMemo(
    () => ({ prefill, token, consume, dismiss }),
    [prefill, token, consume, dismiss],
  );

  return (
    <ImportIntentContext.Provider value={value}>
      {children}
    </ImportIntentContext.Provider>
  );
}

export function useImportIntent(): ImportIntentValue {
  const ctx = useContext(ImportIntentContext);
  if (!ctx) {
    return {
      prefill: null,
      token: 0,
      consume: () => {},
      dismiss: () => {},
    };
  }
  return ctx;
}
