import {
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";

export interface SolidSelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

interface Props {
  value: string;
  options: SolidSelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
  placeholder?: string;
  className?: string;
  /** Show as tall list (e.g. node picker) instead of compact dropdown. */
  list?: boolean;
  listSize?: number;
  "aria-label"?: string;
}

/**
 * Theme-aware select. Native &lt;select&gt; popups on macOS WKWebView are
 * system menus (dark vibrancy) and cannot be restyled — use this instead.
 */
export function SolidSelect({
  value,
  options,
  onChange,
  disabled = false,
  placeholder = "—",
  className = "",
  list = false,
  listSize = 6,
  "aria-label": ariaLabel,
}: Props) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  /** Ignore the synthetic re-click that &lt;label&gt; may fire on the trigger. */
  const ignoreToggleUntil = useRef(0);
  const listId = useId();

  const selected = options.find((o) => o.value === value);
  const label = selected?.label ?? (value ? value : placeholder);

  useEffect(() => {
    if (!open) return;
    function onDoc(e: MouseEvent) {
      const t = e.target as Node | null;
      if (t && rootRef.current?.contains(t)) return;
      setOpen(false);
    }
    function onKey(e: globalThis.KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    // bubble phase so option handlers run first
    document.addEventListener("pointerdown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  function pick(v: string, e?: ReactMouseEvent) {
    e?.preventDefault();
    e?.stopPropagation();
    onChange(v);
    if (!list) {
      ignoreToggleUntil.current = Date.now() + 300;
      setOpen(false);
    }
  }

  function toggleOpen() {
    if (disabled) return;
    if (Date.now() < ignoreToggleUntil.current) return;
    setOpen((o) => !o);
  }

  function onTriggerKey(e: KeyboardEvent) {
    if (disabled) return;
    if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown") {
      e.preventDefault();
      toggleOpen();
    }
  }

  if (list) {
    const rows = Math.min(Math.max(listSize, 3), 12);
    return (
      <div
        ref={rootRef}
        className={`solid-select solid-select-list ${className}`.trim()}
        role="listbox"
        aria-label={ariaLabel}
        aria-activedescendant={value ? `${listId}-${value}` : undefined}
        style={{ maxHeight: `${rows * 2.1}rem` }}
      >
        {options.map((o) => {
          const active = o.value === value;
          return (
            <button
              key={o.value || "__empty"}
              type="button"
              id={o.value ? `${listId}-${o.value}` : undefined}
              role="option"
              aria-selected={active}
              disabled={disabled || o.disabled}
              className={`solid-select-option${active ? " active" : ""}`}
              onMouseDown={(e) => {
                if (disabled || o.disabled) return;
                pick(o.value, e);
              }}
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
              }}
            >
              {o.label}
            </button>
          );
        })}
      </div>
    );
  }

  return (
    <div
      ref={rootRef}
      className={`solid-select ${open ? "open" : ""} ${className}`.trim()}
      onClick={(e) => e.stopPropagation()}
    >
      <button
        type="button"
        className="solid-select-trigger"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={(e) => {
          e.preventDefault();
          e.stopPropagation();
          toggleOpen();
        }}
        onKeyDown={onTriggerKey}
      >
        <span className="solid-select-label">{label}</span>
        <span className="solid-select-caret" aria-hidden />
      </button>
      {open && (
        <div
          className="solid-select-pop"
          role="listbox"
          id={listId}
          onMouseDown={(e) => e.stopPropagation()}
        >
          {options.map((o) => {
            const active = o.value === value;
            return (
              <button
                key={o.value || "__empty"}
                type="button"
                role="option"
                aria-selected={active}
                disabled={o.disabled}
                className={`solid-select-option${active ? " active" : ""}`}
                onMouseDown={(e) => {
                  if (o.disabled) return;
                  pick(o.value, e);
                }}
                onClick={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                }}
              >
                {o.label}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
