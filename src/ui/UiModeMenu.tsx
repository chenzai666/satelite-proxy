import { useCallback, useEffect, useRef, useState } from "react";
import {
  getProxyStatus,
  peekProxyStatus,
  restartProxy,
  setCoreType,
} from "../api";
import type { CoreKind } from "../types";
import { useUiMode, type UiMode } from "./UiModeContext";

/**
 * Top-bar ⋯ quick menu: switch runtime UI mode, switch core (sing-box/Xray),
 * restart core, and copy the proxy env snippet — reachable from any tab.
 * Single ⋯ keeps the navbar tidy (no duplicate capsule switches next to the
 * theme picker).
 */
export function UiModeMenu() {
  const { mode, setMode } = useUiMode();
  const [open, setOpen] = useState(false);
  const [mixedPort, setMixedPort] = useState<number | null>(null);
  const [coreType, setCoreTypeState] = useState<CoreKind>(
    () => peekProxyStatus()?.core_type === "xray" ? "xray" : "singbox",
  );
  const [switchingCore, setSwitchingCore] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [envCopied, setEnvCopied] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  // Refresh mixed_port / core type so actions work without a Dashboard mount.
  const refreshPort = useCallback(async () => {
    const s = await getProxyStatus().catch(() => null);
    if (s?.mixed_port) setMixedPort(s.mixed_port);
    if (s?.core_type === "xray" || s?.core_type === "singbox") {
      setCoreTypeState(s.core_type);
    }
  }, []);

  useEffect(() => {
    void refreshPort();
  }, [refreshPort]);

  // Close on outside click / Escape.
  useEffect(() => {
    if (!open) return;
    function onDoc(e: MouseEvent) {
      const t = e.target as Node | null;
      if (t && rootRef.current?.contains(t)) return;
      setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("pointerdown", onDoc, true);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDoc, true);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  // Auto-clear the ephemeral toast/checkmark.
  useEffect(() => {
    if (!toast) return;
    const id = window.setTimeout(() => setToast(null), 1500);
    return () => window.clearTimeout(id);
  }, [toast]);

  function flash(msg: string) {
    setToast(msg);
    if (msg) {
      window.setTimeout(() => setOpen(false), 600);
    }
  }

  function pick(next: UiMode) {
    setMode(next);
    setOpen(false);
  }

  /** Switch the active core; the backend restarts a running core onto the
   *  new binary (debounced), so a spinner covers the transition. */
  async function onPickCore(kind: CoreKind) {
    if (kind === coreType || switchingCore) return;
    setSwitchingCore(true);
    try {
      await setCoreType(kind);
      setCoreTypeState(kind);
      flash(kind === "xray" ? "已切换到 Xray" : "已切换到 sing-box");
    } catch (e) {
      flash(typeof e === "string" ? e : "切换失败");
    } finally {
      setSwitchingCore(false);
    }
  }

  async function onRestart() {
    if (restarting) return;
    setRestarting(true);
    try {
      await restartProxy();
      flash("内核已重启");
    } catch (e) {
      flash(typeof e === "string" ? e : "重启失败");
    } finally {
      setRestarting(false);
    }
  }

  async function onCopyEnv() {
    const port = mixedPort ?? 2080;
    const proxyUrl = `http://127.0.0.1:${port}`;
    const isWindows = /Windows/i.test(navigator.userAgent);
    const text = isWindows
      ? `$env:ALL_PROXY = "${proxyUrl}"`
      : `export all_proxy=${proxyUrl}`;
    try {
      await navigator.clipboard.writeText(text);
      setEnvCopied(true);
      flash("已复制环境变量");
      window.setTimeout(() => setEnvCopied(false), 1500);
    } catch {
      flash("复制失败");
    }
  }

  return (
    <div className="ui-mode-menu" ref={rootRef} data-ui-mode-menu>
      <button
        type="button"
        className="ui-mode-menu-trigger"
        aria-label="快捷菜单"
        aria-haspopup="menu"
        aria-expanded={open}
        title="快捷菜单"
        onClick={() => {
          setOpen((v) => !v);
          void refreshPort();
        }}
      >
        ⋯
      </button>
      {open && (
        <div className="ui-mode-menu-pop" role="menu">
          <div className="ui-mode-menu-label">运行模式</div>
          <button
            type="button"
            role="menuitemradio"
            aria-checked={mode === "simple"}
            className={`ui-mode-menu-item ${mode === "simple" ? "active" : ""}`}
            onClick={() => pick("simple")}
          >
            <span className="ui-mode-radio" aria-hidden>
              {mode === "simple" ? "●" : "○"}
            </span>
            简洁模式
          </button>
          <button
            type="button"
            role="menuitemradio"
            aria-checked={mode === "pro"}
            className={`ui-mode-menu-item ${mode === "pro" ? "active" : ""}`}
            onClick={() => pick("pro")}
          >
            <span className="ui-mode-radio" aria-hidden>
              {mode === "pro" ? "●" : "○"}
            </span>
            完整模式
          </button>

          <div className="ui-mode-menu-sep" aria-hidden />

          <div className="ui-mode-menu-label">切换内核</div>
          <button
            type="button"
            role="menuitemradio"
            aria-checked={coreType === "singbox"}
            className={`ui-mode-menu-item ${coreType === "singbox" ? "active" : ""}`}
            disabled={switchingCore}
            onClick={() => void onPickCore("singbox")}
          >
            <span className="ui-mode-radio" aria-hidden>
              {switchingCore && coreType !== "singbox" ? (
                <span className="lat-spinner ui-mode-restart-spinner" />
              ) : coreType === "singbox" ? (
                "●"
              ) : (
                "○"
              )}
            </span>
            sing-box
          </button>
          <button
            type="button"
            role="menuitemradio"
            aria-checked={coreType === "xray"}
            className={`ui-mode-menu-item ${coreType === "xray" ? "active" : ""}`}
            disabled={switchingCore}
            onClick={() => void onPickCore("xray")}
          >
            <span className="ui-mode-radio" aria-hidden>
              {switchingCore && coreType !== "xray" ? (
                <span className="lat-spinner ui-mode-restart-spinner" />
              ) : coreType === "xray" ? (
                "●"
              ) : (
                "○"
              )}
            </span>
            Xray
          </button>

          <div className="ui-mode-menu-sep" aria-hidden />

          <button
            type="button"
            role="menuitem"
            className="ui-mode-menu-item"
            disabled={restarting}
            onClick={() => void onRestart()}
          >
            <span className="ui-mode-radio" aria-hidden>
              {restarting ? (
                <span className="lat-spinner ui-mode-restart-spinner" />
              ) : (
                "↻"
              )}
            </span>
            {restarting ? "重启中…" : "重启内核"}
          </button>
          <button
            type="button"
            role="menuitem"
            className="ui-mode-menu-item"
            onClick={() => void onCopyEnv()}
          >
            <span className="ui-mode-radio" aria-hidden>
              {envCopied ? "✓" : "⧉"}
            </span>
            {envCopied ? "已复制" : "复制环境变量"}
          </button>
        </div>
      )}
      {toast && (
        <div className="ui-mode-menu-toast" role="status">
          {toast}
        </div>
      )}
    </div>
  );
}
