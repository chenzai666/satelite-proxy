import type { ThemeId } from "../types";

/** Accent preset: a brand/primary color, with a shade per light/dark theme. */
export interface AccentPreset {
  id: string;
  /** Display name (i18n-independent; shown as the swatch title). */
  name: string;
  /** Base hex for the dark (aerospace) theme — usually lighter/pastel. */
  aerospace: string;
  /** Base hex for the light (day) theme — usually deeper for contrast. */
  day: string;
}

/**
 * Macaron-toned accent presets. The first entry (`green`) is the default and
 * matches the original brand color, so existing users see no change.
 */
export const ACCENTS: AccentPreset[] = [
  { id: "green", name: "薄荷", aerospace: "#55c89a", day: "#1f9a72" },
  { id: "blue", name: "天蓝", aerospace: "#6bb6e8", day: "#2e86c8" },
  { id: "purple", name: "香芋", aerospace: "#b19cd9", day: "#8e5bb8" },
  { id: "pink", name: "蜜桃", aerospace: "#f4a6b8", day: "#d65a7e" },
  { id: "orange", name: "奶橙", aerospace: "#f5b97a", day: "#d88a3d" },
  { id: "cyan", name: "湖蓝", aerospace: "#7ad7d7", day: "#2fa9a9" },
];

export const DEFAULT_ACCENT = "green";

export function defaultAccent(): string {
  return DEFAULT_ACCENT;
}

/** True when `id` is a custom accent stored as a `#rrggbb` hex string. */
export function isCustomHexAccent(id: string | null | undefined): boolean {
  return !!id && /^#[0-9a-f]{6}$/i.test(id.trim());
}

/** Resolve a stored accent id to its preset, falling back to the default.
 *  Custom picker colors are stored verbatim (`#rrggbb`) and resolve to a
 *  virtual preset that uses the same hex for both themes. */
export function resolveAccent(id: string | null | undefined): AccentPreset {
  if (typeof id === "string" && isCustomHexAccent(id)) {
    const hex = id.trim().toLowerCase();
    return { id: hex, name: "Custom", aerospace: hex, day: hex };
  }
  return ACCENTS.find((a) => a.id === id) ?? ACCENTS[0];
}

/** Returns true when `id` is a known accent preset id or a custom hex. */
export function isValidAccent(id: string | null | undefined): id is string {
  return (
    isCustomHexAccent(id) || (!!id && ACCENTS.some((a) => a.id === id))
  );
}

/** Parse a `#rrggbb` hex into an `{ r, g, b }` tuple. Returns null on bad input. */
export function hexToRgb(hex: string): { r: number; g: number; b: number } | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
}

/** `{ r, g, b }` (0–255) → `#rrggbb`, lowercase. */
export function rgbToHex(r: number, g: number, b: number): string {
  const c = (v: number) =>
    Math.max(0, Math.min(255, Math.round(v)))
      .toString(16)
      .padStart(2, "0");
  return `#${c(r)}${c(g)}${c(b)}`;
}

/** RGB → HSV. h in [0,360), s/v in [0,1]. */
export function rgbToHsv(
  r: number,
  g: number,
  b: number,
): { h: number; s: number; v: number } {
  const rr = r / 255,
    gg = g / 255,
    bb = b / 255;
  const max = Math.max(rr, gg, bb),
    min = Math.min(rr, gg, bb);
  const d = max - min;
  let h = 0;
  if (d > 0) {
    if (max === rr) h = 60 * (((gg - bb) / d) % 6);
    else if (max === gg) h = 60 * ((bb - rr) / d + 2);
    else h = 60 * ((rr - gg) / d + 4);
  }
  if (h < 0) h += 360;
  return { h, s: max === 0 ? 0 : d / max, v: max };
}

/** HSV → RGB. h in [0,360), s/v in [0,1]. */
export function hsvToRgb(
  h: number,
  s: number,
  v: number,
): { r: number; g: number; b: number } {
  const c = v * s;
  const hp = (((h % 360) + 360) % 360) / 60;
  const x = c * (1 - Math.abs((hp % 2) - 1));
  let r = 0,
    g = 0,
    b = 0;
  if (hp < 1) [r, g, b] = [c, x, 0];
  else if (hp < 2) [r, g, b] = [x, c, 0];
  else if (hp < 3) [r, g, b] = [0, c, x];
  else if (hp < 4) [r, g, b] = [0, x, c];
  else if (hp < 5) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  const m = v - c;
  return {
    r: Math.round((r + m) * 255),
    g: Math.round((g + m) * 255),
    b: Math.round((b + m) * 255),
  };
}

/** Perceived luminance (Rec. 709 weights), 0–1. */
function luminanceOf(r: number, g: number, b: number): number {
  return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
}

/**
 * Lightness-based pick for on-accent text color (black/white) so labels on a
 * filled primary button stay legible across all presets.
 */
function onColorFor(r: number, g: number, b: number): string {
  return luminanceOf(r, g, b) > 0.6 ? "#0c1210" : "#ffffff";
}

/**
 * Custom picker hexes have no per-theme shade (presets ship a lighter dark-
 * theme and a deeper light-theme variant), so one raw pick could land as dark
 * accent text on dark glass. Nudge the *applied* color into the theme's
 * readable lightness band — the stored id stays verbatim, and preset colors
 * are designer-tuned and bypass this entirely.
 */
function clampForTheme(
  r: number,
  g: number,
  b: number,
  theme: ThemeId,
): { r: number; g: number; b: number } {
  const step = 0.08;
  if (theme === "day") {
    // Light theme: accent text sits on light surfaces — darken until ≤ 0.6.
    for (let i = 0; i < 24 && luminanceOf(r, g, b) > 0.6; i++) {
      r *= 1 - step;
      g *= 1 - step;
      b *= 1 - step;
    }
  } else {
    // Dark theme: lift toward white until luminance ≥ 0.5.
    for (let i = 0; i < 24 && luminanceOf(r, g, b) < 0.5; i++) {
      r += (255 - r) * step;
      g += (255 - g) * step;
      b += (255 - b) * step;
    }
  }
  return { r: Math.round(r), g: Math.round(g), b: Math.round(b) };
}

/**
 * Override the primary CSS variables on :root so the whole UI re-skins
 * to the chosen accent. Derives the translucent variants (muted/glow/border)
 * from the single base hex via rgba(). Call whenever theme OR accent changes.
 * `--success` is intentionally NOT touched: it is a fixed semantic green
 * (defined per theme in App.css tokens) so ok/direct/latency states keep
 * their meaning no matter which accent is active.
 */
export function applyAccentToDom(
  accentId: string | null | undefined,
  theme: ThemeId,
): void {
  const preset = resolveAccent(accentId);
  const base = hexToRgb(preset[theme]);
  if (!base) return;
  let { r, g, b } = base;
  if (isCustomHexAccent(accentId)) {
    ({ r, g, b } = clampForTheme(r, g, b, theme));
  }
  const rgb = (a: number) => `rgba(${r}, ${g}, ${b}, ${a})`;

  // Hover: lighten ~8% toward white. Cheap approximation good enough for swatches.
  const mix = (t: number) => ({
    r: Math.round(r + (255 - r) * t),
    g: Math.round(g + (255 - g) * t),
    b: Math.round(b + (255 - b) * t),
  });
  const hv = mix(0.12);

  const root = document.documentElement.style;
  root.setProperty("--primary", rgbToHex(r, g, b));
  root.setProperty("--primary-hover", `rgb(${hv.r}, ${hv.g}, ${hv.b})`);
  root.setProperty("--primary-muted", rgb(0.14));
  root.setProperty("--primary-glow", rgb(theme === "day" ? 0.2 : 0.28));
  root.setProperty("--primary-border", rgb(0.35));
  root.setProperty("--primary-border-strong", rgb(theme === "day" ? 0.5 : 0.55));
  root.setProperty("--on-primary", onColorFor(r, g, b));
}

/** Valid glow setting values: `"accent"` (follow the UI accent) or any accent id. */
export function isValidGlow(id: string | null | undefined): id is string {
  return id === "accent" || isValidAccent(id);
}

/** Resolve a stored glow id, falling back to `"accent"` (follow). */
export function normalizeGlowId(id: string | null | undefined): string {
  return isValidGlow(id) ? (id as string) : "accent";
}

/**
 * Scale an rgb tuple toward black until its perceived luminance drops to
 * `target` (mixing toward black scales luminance linearly). Never lightens:
 * already-dark colors pass through untouched.
 */
function deepenToLuminance(
  r: number,
  g: number,
  b: number,
  target: number,
): { r: number; g: number; b: number } {
  const lum = luminanceOf(r, g, b);
  const t = lum > target && lum > 0 ? target / lum : 1;
  return {
    r: Math.round(r * t),
    g: Math.round(g * t),
    b: Math.round(b * t),
  };
}

/**
 * Publish `--glow-rgb` for the background halo layers (app-shell atmosphere +
 * hero glow in App.css). Independent from the accent: `glowId` may be
 * `"accent"` to track it, any preset id, or a custom `#rrggbb` (presets pick
 * their per-theme shade; custom hexes are used verbatim — a wash behind
 * content needs no text-contrast clamp). Re-apply on theme change, and on
 * accent change while following.
 *
 * Also emits `--glow-deep-rgb`: the same hue luminance-normalized for the
 * app-shell wash (0.22 dark / 0.50 light theme — the dark target sits below
 * the original 0.30 baseline for a calmer atmosphere) so the big wash keeps
 * a consistent, subdued brightness whatever glow color is picked — a raw
 * pastel there made dark mode read noticeably brighter. `--hero-glow` keeps
 * the raw variant (it always used the bright accent).
 */
export function applyGlowToDom(
  glowId: string | null | undefined,
  accentId: string | null | undefined,
  theme: ThemeId,
): void {
  const preset = resolveAccent(
    glowId && glowId !== "accent" ? glowId : accentId,
  );
  const base = hexToRgb(preset[theme]);
  if (!base) return;
  const deep = deepenToLuminance(
    base.r,
    base.g,
    base.b,
    theme === "day" ? 0.5 : 0.22,
  );
  const root = document.documentElement.style;
  root.setProperty("--glow-rgb", `${base.r}, ${base.g}, ${base.b}`);
  root.setProperty("--glow-deep-rgb", `${deep.r}, ${deep.g}, ${deep.b}`);
}
