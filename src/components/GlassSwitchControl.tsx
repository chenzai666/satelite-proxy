import {
  useEffect,
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
  const canAnimate = useGlassSwitchAnimation(ready);

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
        if (!event.defaultPrevented) onChange(!checked);
      }}
    >
      <GlassSwitchTrack checked={checked} size={size} animate={canAnimate} />
    </button>
  );
}
