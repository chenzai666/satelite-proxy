import type { CSSProperties } from "react";

interface Option {
  value: string;
  label: string;
}

interface Props {
  value: string;
  options: Option[];
  onChange: (value: string) => void;
  ariaLabel?: string;
  disabled?: boolean;
  /** Per-option disable (e.g. gating TUN when no nodes). */
  disabledValues?: Set<string>;
  /** Per-option title tooltip. */
  titles?: Record<string, string>;
}

/**
 * Three-way glass segmented control. The active option is marked by a sliding
 * frosted-glass capsule (same material as the navbar) that travels between
 * positions; re-used across the dashboard quick controls.
 */
export function GlassSeg({
  value,
  options,
  onChange,
  ariaLabel,
  disabled,
  disabledValues,
  titles,
}: Props) {
  const index = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  );
  return (
    <div
      className="glass-seg"
      role="group"
      aria-label={ariaLabel}
      style={{ "--count": options.length } as CSSProperties}
    >
      <span
        className="glass-seg-indicator"
        aria-hidden="true"
        style={{ transform: `translateX(${index * 100}%)` }}
      />
      {options.map((o) => {
        const isDisabled = disabled || disabledValues?.has(o.value);
        return (
          <button
            key={o.value}
            type="button"
            className={`glass-seg-btn ${value === o.value ? "active" : ""}`}
            disabled={isDisabled}
            title={titles?.[o.value]}
            onClick={() => onChange(o.value)}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
