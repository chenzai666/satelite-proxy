import { useEffect, useRef, useState } from "react";
import { useUiMode, type UiMode } from "./UiModeContext";

/** Top-bar ⋯ menu: pick 简洁 / 完整 runtime UI mode. */
export function UiModeMenu() {
  const { mode, setMode } = useUiMode();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

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

  function pick(next: UiMode) {
    setMode(next);
    setOpen(false);
  }

  return (
    <div className="ui-mode-menu" ref={rootRef} data-ui-mode-menu>
      <button
        type="button"
        className="ui-mode-menu-trigger"
        aria-label="更多"
        aria-haspopup="menu"
        aria-expanded={open}
        title="运行模式"
        onClick={() => setOpen((v) => !v)}
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
        </div>
      )}
    </div>
  );
}
