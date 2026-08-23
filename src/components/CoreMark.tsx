import type { CoreKind } from "../types";

/**
 * Monochrome core monograms — cube (sing-box), lightning bolt (Xray), cat
 * head (meow). Inline SVG with `currentColor` so they inherit the tile tint
 * exactly like the old letter glyphs did; text symbols like ⚡/🐱 would fall
 * back to colored emoji in the WebView and break the muted-mark look.
 */
export function CoreMark({ kind }: { kind: CoreKind }) {
  if (kind === "xray") {
    return (
      <svg viewBox="0 0 16 16" aria-hidden focusable="false">
        <path
          fill="currentColor"
          d="M9.1 1.2 3.3 9.2h3.8l-.8 5.6 6.5-8.6H8.6Z"
        />
      </svg>
    );
  }
  if (kind === "meow") {
    // Cute chibi cat head: one continuous silhouette (short round ears
    // flowing into a chubby, wider-than-tall face) with two round eye dots
    // knocked out via fill-rule="evenodd".
    return (
      <svg viewBox="0 0 16 16" aria-hidden focusable="false">
        <path
          fill="currentColor"
          fillRule="evenodd"
          d="M6.85 5.4C6.02 4.55 5.47 3.7 4.69 3.23C4.28 2.95 3.73 3.05 3.64 3.46C3.42 4.32 3.34 5.5 3.46 6.54C3.22 7.39 3.13 8.28 3.13 9.18C3.13 11.73 5.44 13.53 8 13.53C10.56 13.53 12.87 11.73 12.87 9.18C12.87 8.28 12.78 7.39 12.54 6.54C12.66 5.5 12.58 4.32 12.36 3.46C12.27 3.05 11.72 2.95 11.31 3.23C10.53 3.7 9.98 4.55 9.15 5.4C8.75 5.22 8.35 5.16 8 5.16C7.65 5.16 7.25 5.22 6.85 5.4ZM5.88 7.5a1.06 0.79 0 0 1 0 1.58a1.06 0.79 0 0 1 0-1.58ZM10.12 7.5a1.06 0.79 0 0 1 0 1.58a1.06 0.79 0 0 1 0-1.58Z"
        />
      </svg>
    );
  }
  // sing-box: isometric cube (hexagon silhouette + inner edges).
  return (
    <svg viewBox="0 0 16 16" aria-hidden focusable="false">
      <g
        fill="none"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinejoin="round"
        strokeLinecap="round"
      >
        <path d="M8 1.7 13.8 5v6L8 14.3 2.2 11V5Z" />
        <path d="M2.2 5 8 8.4 13.8 5M8 8.4v5.9" />
      </g>
    </svg>
  );
}
