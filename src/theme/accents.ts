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

/**
 * Lightness-based pick for on-accent text color (black/white) so labels on a
 * filled primary button stay legible across all presets.
 */
function onColorFor(r: number, g: number, b: number): string {
  // Perceived luminance (Rec. 709 weights).
  const lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  return lum > 0.6 ? "#0c1210" : "#ffffff";
}

/**
 * Override the primary/success CSS variables on :root so the whole UI re-skins
 * to the chosen accent. Derives the translucent variants (muted/glow/border)
 * from the single base hex via rgba(). Call whenever theme OR accent changes.
 */
export function applyAccentToDom(
  accentId: string | null | undefined,
  theme: ThemeId,
): void {
  const preset = resolveAccent(accentId);
  const base = hexToRgb(preset[theme]);
  if (!base) return;
  const { r, g, b } = base;
  const rgb = (a: number) => `rgba(${r}, ${g}, ${b}, ${a})`;

  // Hover: lighten ~8% toward white. Cheap approximation good enough for swatches.
  const mix = (t: number) => ({
    r: Math.round(r + (255 - r) * t),
    g: Math.round(g + (255 - g) * t),
    b: Math.round(b + (255 - b) * t),
  });
  const hv = mix(0.12);

  const root = document.documentElement.style;
  root.setProperty("--primary", preset[theme]);
  root.setProperty("--primary-hover", `rgb(${hv.r}, ${hv.g}, ${hv.b})`);
  root.setProperty("--primary-muted", rgb(0.14));
  root.setProperty("--primary-glow", rgb(theme === "day" ? 0.2 : 0.28));
  root.setProperty("--primary-border", rgb(0.35));
  root.setProperty("--primary-border-strong", rgb(theme === "day" ? 0.5 : 0.55));
  root.setProperty("--on-primary", onColorFor(r, g, b));
  root.setProperty("--success", preset[theme]);
  root.setProperty("--success-muted", rgb(0.14));
}
