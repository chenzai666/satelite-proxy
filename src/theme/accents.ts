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

/** Resolve a stored accent id to its preset, falling back to the default. */
export function resolveAccent(id: string | null | undefined): AccentPreset {
  return ACCENTS.find((a) => a.id === id) ?? ACCENTS[0];
}

/** Returns true when `id` is a known accent preset id. */
export function isValidAccent(id: string | null | undefined): id is string {
  return !!id && ACCENTS.some((a) => a.id === id);
}

/** Parse a `#rrggbb` hex into an `{ r, g, b }` tuple. Returns null on bad input. */
function hexToRgb(hex: string): { r: number; g: number; b: number } | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
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
