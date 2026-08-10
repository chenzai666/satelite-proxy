import type { ButtonHTMLAttributes, ReactNode } from "react";

type Variant = "plain" | "primary" | "danger";

interface Props
  extends Omit<
    ButtonHTMLAttributes<HTMLButtonElement>,
    "type" | "className"
  > {
  /** Leading icon/glyph (emoji or short text). */
  icon?: ReactNode;
  /** Visual treatment. `primary` = accent-tinted glass, `danger` = red-tinted. */
  variant?: Variant;
  /** Show only the icon (no children) with tighter padding. */
  iconOnly?: boolean;
}

/**
 * Standalone glass capsule button — same frosted material as the GlassSeg
 * active indicator and the navbar. Re-usable across pages.
 *
 * Renders a real <button type="button"> with the `.glass-btn` class so the
 * global button reset (which excludes a few known classes) leaves it alone.
 */
export function GlassButton({
  icon,
  variant = "plain",
  iconOnly = false,
  children,
  disabled,
  title,
  onClick,
  ...rest
}: Props) {
  const cls = [
    "glass-btn",
    variant === "plain" ? "" : variant,
    iconOnly ? "icon-only" : "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <button
      {...rest}
      type="button"
      className={cls}
      title={title}
      disabled={disabled}
      onClick={onClick}
    >
      {icon ? <span className="glass-btn-icon" aria-hidden>{icon}</span> : null}
      {children ? <span className="glass-btn-label">{children}</span> : null}
    </button>
  );
}
