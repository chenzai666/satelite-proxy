import type { ReactNode } from "react";
import {
  GlassSwitchControl,
  GlassSwitchTrack,
  type GlassSwitchSize,
  useGlassSwitchAnimation,
} from "./GlassSwitchControl";

interface Props {
  checked: boolean;
  onChange: (next: boolean) => void;
  /** Inline label rendered next to the switch. */
  label?: ReactNode;
  /** Accessible label / tooltip. */
  title?: string;
  disabled?: boolean;
  /**
   * Wrap the label + switch in one frosted capsule (same glass material as
   * GlassButton), so the whole control reads as a single glass button that
   * also toggles. Default: false (bare track, like the settings switches).
   */
  capsule?: boolean;
  /** Switch track size: `md` (default, 40×22) or `sm` (32×18, ~80%). */
  size?: GlassSwitchSize;
  /** False while the parent is still loading the initial persisted value. */
  ready?: boolean;
}

/**
 * Glass-material toggle switch — a frosted capsule track with a sliding
 * frosted thumb. Mirrors the navbar / GlassSeg material so it reads as part
 * of the same control family.
 *
 * With `capsule`, the label and track sit inside one shared frosted pill
 * (like a GlassButton that also toggles) — useful in toolbars where the
 * label + switch should read as one affordance.
 *
 * The thumb is plain glass when off and tinted with the accent when on, so
 * "on" still reads as primary without abandoning the glass look.
 */
export function GlassSwitch({
  checked,
  onChange,
  label,
  title,
  disabled,
  capsule = false,
  size = "md",
  ready = true,
}: Props) {
  const canAnimate = useGlassSwitchAnimation(ready);

  if (capsule) {
    return (
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        className={`glass-btn glass-switch-capsule${checked ? " on" : ""}${
          canAnimate ? "" : " no-anim"
        }`}
        title={title}
        disabled={disabled}
        onClick={() => onChange(!checked)}
      >
        {label != null && <span className="glass-switch-label">{label}</span>}
        <GlassSwitchTrack checked={checked} size={size} animate={canAnimate} />
      </button>
    );
  }

  return (
    <div
      className={`glass-switch-row${disabled ? " disabled" : ""}`}
      title={title}
    >
      {label != null && <span className="glass-switch-label">{label}</span>}
      <GlassSwitchControl
        checked={checked}
        onChange={onChange}
        title={title}
        disabled={disabled}
        size={size}
        ready={ready}
      />
    </div>
  );
}
