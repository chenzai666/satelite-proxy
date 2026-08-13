import type { ButtonHTMLAttributes, ReactNode } from "react";

type Variant = "plain" | "primary" | "danger";

interface Props
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "className"> {
  /** Leading icon/glyph (emoji or short text). */
  icon?: ReactNode;
  /** Visual treatment. `primary` = accent-tinted glass, `danger` = red-tinted. */
  variant?: Variant;
  /** Show only the icon (no children) with tighter padding. */
  iconOnly?: boolean;
  /** Extra class on the root, for callers that need to scope a layout tweak. */
  className?: string;
}

/**
 * Standalone glass capsule button — same frosted material as the GlassSeg
 * active indicator and the navbar. Re-usable across pages.
 *
 * Renders a real <button> with the `.glass-btn` class so the global button
 * reset (which excludes a few known classes) leaves it alone. Defaults to
 * `type="button"` like a plain <button> outside a form would want, but form
 * footers can pass `type="submit"`.
 */
export function GlassButton({
  icon,
  variant = "plain",
  iconOnly = false,
  className,
  children,
  disabled,
  title,
  onClick,
  type = "button",
  ...rest
}: Props) {
  const cls = [
    "glass-btn",
    variant === "plain" ? "" : variant,
    iconOnly ? "icon-only" : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <button
      {...rest}
      type={type}
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
