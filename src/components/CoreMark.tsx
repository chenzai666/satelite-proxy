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
    // Pointy-eared head silhouette (ears overlap the head circle).
    return (
      <svg viewBox="0 0 16 16" aria-hidden focusable="false">
        <path fill="currentColor" d="M3.1 7.1 2.7 1.9l4.6 2.9Z" />
        <path fill="currentColor" d="M12.9 7.1l.4-5.2-4.6 2.9Z" />
        <circle cx="8" cy="9.9" r="4.7" fill="currentColor" />
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
