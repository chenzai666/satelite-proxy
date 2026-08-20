import {
  useEffect,
  useRef,
  useState,
  type MouseEventHandler,
} from "react";

export type GlassSwitchSize = "md" | "sm";

interface TrackProps {
  checked: boolean;
  size?: GlassSwitchSize;
  animate?: boolean;
}

interface Props {
  checked: boolean;
  onChange: (next: boolean) => void;
  title?: string;
  disabled?: boolean;
  size?: GlassSwitchSize;
  /** False while the parent is loading the initial persisted value. */
  ready?: boolean;
  /** Optional click hook for callers that need to stop event propagation. */
  onClick?: MouseEventHandler<HTMLButtonElement>;
}

/** Keep the initial/persisted state from animating in from the off position. */
export function useGlassSwitchAnimation(ready: boolean) {
  const [canAnimate, setCanAnimate] = useState(false);

  useEffect(() => {
    if (!ready) {
      setCanAnimate(false);
      return;
    }

    let nextRaf = 0;
    const paintRaf = requestAnimationFrame(() => {
      nextRaf = requestAnimationFrame(() => setCanAnimate(true));
    });
    return () => {
      cancelAnimationFrame(paintRaf);
      cancelAnimationFrame(nextRaf);
    };
  }, [ready]);

  return canAnimate;
}

/**
 * Thumb animation gate shared by GlassSwitchControl and GlassSwitch's capsule
 * variant. Mirrors GlassSeg: the thumb only slides for user-driven changes —
 * a persisted value landing after mount (or a poll refresh) paints its
 * position directly instead of sliding in from the default.
 */
export function useGlassSwitchThumb(checked: boolean, ready: boolean) {
  const canAnimate = useGlassSwitchAnimation(ready);
  const committedCheckedRef = useRef(checked);
  const pendingUserCheckedRef = useRef<boolean | null>(null);

  useEffect(() => {
    committedCheckedRef.current = checked;
    if (pendingUserCheckedRef.current === checked) {
      pendingUserCheckedRef.current = null;
    }
  }, [checked]);

  const positionChanged = committedCheckedRef.current !== checked;
  const isUserChange = pendingUserCheckedRef.current === checked;

  return {
    animate: canAnimate && (!positionChanged || isUserChange),
    /** Call right before onChange from a user click. */
    markUserChange() {
      pendingUserCheckedRef.current = !checked;
    },
  };
}

/** Visual glass track, reusable inside larger composite controls. */
export function GlassSwitchTrack({
  checked,
  size = "md",
  animate = true,
}: TrackProps) {
  return (
    <span
      className={`glass-switch-track${size === "sm" ? " sm" : ""}${
        checked ? " on" : ""
      }${animate ? "" : " no-anim"}`}
      aria-hidden="true"
    >
      <span className="glass-switch-thumb" />
    </span>
  );
}

/** Standalone interactive switch using the track from the labeled GlassSwitch. */
export function GlassSwitchControl({
  checked,
  onChange,
  title,
  disabled,
  size = "md",
  ready = true,
  onClick,
}: Props) {
  const { animate, markUserChange } = useGlassSwitchThumb(checked, ready);

  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={title}
      className="glass-switch"
      title={title}
      disabled={disabled}
      onClick={(event) => {
        onClick?.(event);
        if (!event.defaultPrevented) {
          markUserChange();
          onChange(!checked);
        }
      }}
    >
      <GlassSwitchTrack checked={checked} size={size} animate={animate} />
    </button>
  );
}
