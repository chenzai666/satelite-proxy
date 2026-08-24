import { useEffect, useRef, useState, type ReactNode } from "react";

/**
 * Faithful Canvas2D port of canvasui's decrypt-reveal (MIT, DavidHDev/canvas-ui).
 *
 * What was ported 1:1 from the original WebGL2 shaders:
 * - glyph shape matching: 95 printable-ASCII glyphs pre-rendered into an
 *   atlas, each reduced to a 6-value shape vector (6 sample circles); every
 *   content cell samples the same 6 circles + an nx×ny grid-max pass and
 *   picks the best-matching glyph by SSD.
 * - cipher glyph color: the underlying cell color is normalized ("vivid") to
 *   ~0.75 luminance against the background, pushed toward ink when too dark,
 *   i.e. glyphs inherit the content's colors — not a flat green.
 * - idle scramble: per-(cell, tick) hash decides glyph swaps at
 *   scramble×0.35 probability / scrambleSpeed per second.
 * - decrypt circle: full reveal inside radius×(1−softness), smoothstep
 *   feather to radius, Gaussian ring (σ = radius×edgeWidth/2, min 6px)
 *   centered at 75% of the radius drives flicker, green edgeTint and the
 *   edgeGlow brightness surge; cursor follow and enter/leave are damped by
 *   the same exponential smoothing.
 * - encrypted base: background mixed with passthrough of the real content.
 *
 * Not ported (WebGL-only, visually minor): chromatic aberration at the
 * wavefront, per-pixel (vs per-cell) feathering, glyph mipmaps.
 */

// ---- constants from the original source ----
const CHARSET = Array.from({ length: 95 }, (_, i) => String.fromCharCode(32 + i));
const LW = [0.299, 0.587, 0.114];
const INNER_CIRCLES: Array<[number, number]> = [
  [0.3, 0.32],
  [0.7, 0.32],
  [0.28, 0.52],
  [0.72, 0.52],
  [0.32, 0.72],
  [0.68, 0.72],
];
const RING_6 = Array.from({ length: 6 }, (_, i) => {
  const a = (i / 6) * Math.PI * 2;
  return [Math.cos(a), -Math.sin(a)] as [number, number];
});
const OUTER_TAPS: Array<[number, number]> = [
  [0.08, 0.2],
  [0.5, 0.14],
  [0.92, 0.2],
  [0.06, 0.5],
  [0.94, 0.5],
  [0.08, 0.8],
  [0.5, 0.86],
  [0.92, 0.8],
  [0.22, 0.5],
  [0.78, 0.5],
];
const FONT_STACK = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace";

const clamp = (x: number, a: number, b: number) => Math.min(b, Math.max(a, x));
const smoothstep = (a: number, b: number, x: number) => {
  const t = clamp((x - a) / (b - a), 0, 1);
  return t * t * (3 - 2 * t);
};
const fract = (x: number) => x - Math.floor(x);
/** Port of the shader's hash — same cadence for glyph churn. */
function ghash(x: number, y: number): number {
  const p = fract(x * 127.1 + y * 311.7);
  return fract(Math.sin(p * 43758.545) * 1e4);
}

function parseColor(hex: string): [number, number, number] {
  const h = hex.replace("#", "");
  const n = parseInt(h.length === 3 ? h.replace(/./g, (c) => c + c) : h, 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

/** Port of buildGlyphAtlas' shape computation (atlas stays on a canvas). */
function buildGlyphAtlas(): { chars: string[]; shapes: number[][] } {
  const cellH = 64;
  const cellW = Math.max(8, Math.round(cellH * 0.75));
  const pad = 8;
  const adv = cellW + pad * 2;
  const count = CHARSET.length;
  const fontPx = Math.min(cellH * 0.92, cellW / 0.58);

  const atlas = document.createElement("canvas");
  atlas.width = adv * count;
  atlas.height = cellH + pad * 2;
  const ctx = atlas.getContext("2d", { willReadFrequently: true })!;
  ctx.font = `600 ${fontPx}px ${FONT_STACK}`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillStyle = "#fff";
  CHARSET.forEach((ch, i) => ctx.fillText(ch, (i + 0.5) * adv, atlas.height / 2));

  const data = ctx.getImageData(0, 0, atlas.width, atlas.height).data;
  const at = (x: number, y: number) => {
    const xi = clamp(Math.round(x), 0, atlas.width - 1);
    const yi = clamp(Math.round(y), 0, atlas.height - 1);
    return data[(yi * atlas.width + xi) * 4 + 3] / 255;
  };
  // Circle-average alpha, 48 taps (ported sampling density).
  const circleAvg = (cx: number, cy: number, r: number) => {
    let acc = 0;
    for (let i = 0; i < 48; i++) {
      const a = (i / 48) * Math.PI * 2;
      acc += at(cx + Math.cos(a) * r, cy + Math.sin(a) * r);
    }
    return acc / 48;
  };

  const shapes: number[][] = CHARSET.map((_, i) => {
    const ox = i * adv + pad;
    const oy = pad;
    return INNER_CIRCLES.map(([nx, ny]) =>
      circleAvg(ox + nx * cellW, oy + ny * cellH, cellH * 0.26),
    );
  });
  // Per-circle normalize by the peak across glyphs (ported).
  for (let c = 0; c < 6; c++) {
    let peak = 0;
    for (let g = 0; g < count; g++) peak = Math.max(peak, shapes[g][c]);
    if (peak > 0) for (let g = 0; g < count; g++) shapes[g][c] /= peak;
  }
  return { chars: CHARSET, shapes };
}

interface CellInfo {
  glyph: number; // -1 = empty (below threshold)
  color: [number, number, number]; // vivid cipher ink
}

export interface DecryptRevealProps {
  /** Content hidden behind the cipher; give it explicit CSS size. */
  children: ReactNode;
  /** Cipher cell height in CSS px; width = cell × aspect. */
  cell?: number;
  /** Cell width / height ratio. */
  aspect?: number;
  /** Decrypt radius in px. */
  radius?: number;
  /** Fraction of the radius that feathers (0–1). */
  softness?: number;
  /** 0 = monochrome `color`, 1 = inherit content colors. */
  colored?: number;
  /** Monochrome cipher color. */
  color?: string;
  brightness?: number;
  legibility?: number;
  contrast?: number;
  exposure?: number;
  /** Fraction of idle cells that keep rerolling. */
  scramble?: number;
  /** Idle reroll ticks per second. */
  scrambleSpeed?: number;
  /** Gaussian wavefront band width, fraction of radius. */
  edgeWidth?: number;
  edgeFlicker?: number;
  edgeGlow?: number;
  edgeTint?: number;
  /** Real-content bleed through the cipher (0–1). */
  passthrough?: number;
  /** Cells with peak signal below this stay empty. */
  threshold?: number;
  /** Cipher background color. */
  background?: string;
  /** Cursor-follow damping in seconds. */
  smoothing?: number;
  /** After the pointer sweeps over and leaves, fade the whole cipher layer
   * out instead of re-encrypting (the content stays revealed). */
  dismissOnLeave?: boolean;
  className?: string;
}

export function DecryptReveal({
  children,
  cell = 10,
  aspect = 0.75,
  radius = 400,
  softness = 0.5,
  colored = 1,
  color = "#4ade80",
  brightness = 1,
  legibility = 1,
  contrast = 1,
  exposure = 1,
  scramble = 0.1,
  scrambleSpeed = 6,
  edgeWidth = 0.2,
  edgeFlicker = 1,
  edgeGlow = 2,
  edgeTint = 0.75,
  passthrough = 0.15,
  threshold = 0.025,
  background = "#000000",
  smoothing = 0.2,
  dismissOnLeave = false,
  className = "",
}: DecryptRevealProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const [box, setBox] = useState<{ w: number; h: number } | null>(null);
  /** Bumped when the inner <img> finishes loading so the canvas effect
   * can sample it. The image itself is always visible underneath. */
  const [imgTick, setImgTick] = useState(0);

  useEffect(() => {
    const el = hostRef.current;
    if (!el) return;
    const measure = () => {
      const r = el.getBoundingClientRect();
      if (r.width <= 0 || r.height <= 0) return;
      setBox((prev) =>
        prev && prev.w === r.width && prev.h === r.height
          ? prev
          : { w: r.width, h: r.height },
      );
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const el = hostRef.current;
    if (!el) return;
    const img = el.querySelector("img");
    if (!img || img.complete) return;
    const bump = () => setImgTick((n) => n + 1);
    img.addEventListener("load", bump);
    img.addEventListener("error", bump);
    return () => {
      img.removeEventListener("load", bump);
      img.removeEventListener("error", bump);
    };
  }, []);

  useEffect(() => {
    if (!box) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const host = hostRef.current;
    if (!host) return;
    const img = host.querySelector("img");
    // Image not painted yet — keep the real <img> visible; this effect
    // re-runs via imgTick once load fires. Never cover it with a stuck
    // black backdrop.
    if (!img || !img.complete || img.naturalWidth === 0) return;

    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const cw = cell * aspect;
    const ch = cell;
    const cols = Math.ceil(box.w / cw);
    const rows = Math.ceil(box.h / ch);

    // Content sampled at device resolution.
    const content = document.createElement("canvas");
    content.width = Math.round(box.w * dpr);
    content.height = Math.round(box.h * dpr);
    const cc = content.getContext("2d", { willReadFrequently: true })!;
    cc.drawImage(img, 0, 0, content.width, content.height);
    const cdata = cc.getImageData(0, 0, content.width, content.height).data;
    const cwPx = Math.round(cw * dpr);
    const chPx = Math.round(ch * dpr);

    const bg = parseColor(background);
    const mono = parseColor(color);

    // tapLevel port (bg=black, opaque content): |rgb-bg| luma × alpha,
    // scaled up to 8× when the channel sum is tiny.
    const tapLevel = (x: number, y: number): [number, number, number] => {
      const xi = clamp(Math.round(x), 0, content.width - 1);
      const yi = clamp(Math.round(y), 0, content.height - 1);
      const o = (yi * content.width + xi) * 4;
      const rgb: [number, number, number] = [
        cdata[o] / 255 - bg[0],
        cdata[o + 1] / 255 - bg[1],
        cdata[o + 2] / 255 - bg[2],
      ];
      const a = cdata[o + 3] / 255;
      const l =
        (Math.abs(rgb[0]) * LW[0] +
          Math.abs(rgb[1]) * LW[1] +
          Math.abs(rgb[2]) * LW[2]) *
        a;
      const dot = rgb[0] * LW[0] + rgb[1] * LW[1] + rgb[2] * LW[2];
      const f = Math.min(l / Math.max(Math.abs(dot), 1e-3), 8);
      return [rgb[0] * f, rgb[1] * f, rgb[2] * f];
    };
    const level = (l: [number, number, number]) =>
      l[0] * LW[0] + l[1] * LW[1] + l[2] * LW[2];
    const sampleCircle = (
      cx: number,
      cy: number,
      r: number,
    ): { rgb: [number, number, number]; a: number } => {
      const acc: [number, number, number] = [0, 0, 0];
      let a = 0;
      const taps: Array<[number, number]> = [[cx, cy]];
      for (const [dx, dy] of RING_6) taps.push([cx + dx * r, cy + dy * r]);
      for (const [tx, ty] of taps) {
        const l = tapLevel(tx, ty);
        acc[0] += l[0];
        acc[1] += l[1];
        acc[2] += l[2];
        a += 1;
      }
      return { rgb: [acc[0] / 7, acc[1] / 7, acc[2] / 7], a: a / 7 };
    };

    const { chars, shapes } = buildGlyphAtlas();

    // ---- cell pass (ported): shape vector + color per cell ----
    const cellsInfo: CellInfo[] = [];
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const ox = c * cwPx;
        const oy = r * chPx;
        const v = [0, 0, 0, 0, 0, 0];
        const acc: [number, number, number] = [0, 0, 0];
        let accA = 0;
        for (let i = 0; i < 6; i++) {
          const [nx, ny] = INNER_CIRCLES[i];
          const s = sampleCircle(
            ox + nx * cwPx,
            oy + ny * chPx,
            chPx * 0.161,
          );
          acc[0] += s.rgb[0];
          acc[1] += s.rgb[1];
          acc[2] += s.rgb[2];
          accA += s.a;
          v[i] = clamp(level(s.rgb) * exposure, 0, 1);
        }
        const avgCol: [number, number, number] = [
          acc[0] / Math.max(accA, 1e-4),
          acc[1] / Math.max(accA, 1e-4),
          acc[2] / Math.max(accA, 1e-4),
        ];
        // Outer taps → direction contrast (identity at contrast = 1).
        let minL = 1;
        let maxL = 0;
        for (const [nx, ny] of OUTER_TAPS) {
          const l = clamp(level(tapLevel(ox + nx * cwPx, oy + ny * chPx)) * exposure, 0, 1);
          minL = Math.min(minL, l);
          maxL = Math.max(maxL, l);
        }
        const peakO = Math.max(0.05, maxL);
        const dirAdj = Math.min(clamp(1 - (maxL - minL) / peakO, 0.4, 1), Math.min(1, minL * 1.6));
        for (let i = 0; i < 6; i++) v[i] *= dirAdj;
        // Grid-max pass.
        const nx = clamp(Math.floor(cwPx), 6, 20);
        const ny = clamp(Math.floor(chPx), 8, 32);
        const gm = [0, 0, 0, 0, 0, 0];
        let inkLev = 0;
        let inkCol: [number, number, number] = [0, 0, 0];
        for (let gy = 0; gy < ny; gy++) {
          for (let gx = 0; gx < nx; gx++) {
            const px = (gx + 0.5) / nx;
            const py = (gy + 0.5) / ny;
            const l = tapLevel(ox + px * cwPx, oy + py * chPx);
            const lev = clamp(level(l) * exposure, 0, 1);
            if (lev > inkLev) {
              inkLev = lev;
              inkCol = l;
            }
            const idx =
              py < 0.41 ? (px < 0.5 ? 0 : 1) : py < 0.68 ? (px < 0.5 ? 2 : 3) : px < 0.5 ? 4 : 5;
            gm[idx] = Math.max(gm[idx], lev);
          }
        }
        for (let i = 0; i < 6; i++) v[i] = Math.max(v[i], clamp(gm[i] * exposure, 0, 1));
        const peak = Math.max(...v);
        if (peak < threshold) {
          cellsInfo.push({ glyph: -1, color: [0, 0, 0] });
          continue;
        }
        const sharp = smoothstep(threshold, threshold + 0.09, peak);
        const solid = step05(peak);
        const lift = (1 - sharp) * (1 - solid);
        const lifted = peak + (1 - peak) * lift;
        for (let i = 0; i < 6; i++) {
          v[i] = clamp(v[i] / (Math.pow(peak, contrast) * lifted), 0, 1);
        }
        const cellCol: [number, number, number] = [
          avgCol[0] + (inkCol[0] - avgCol[0]) * lift,
          avgCol[1] + (inkCol[1] - avgCol[1]) * lift,
          avgCol[2] + (inkCol[2] - avgCol[2]) * lift,
        ];
        // Best-matching glyph by SSD.
        let glyph = 0;
        let bd = Infinity;
        for (let g = 0; g < shapes.length; g++) {
          let d = 0;
          for (let i = 0; i < 6; i++) d += (v[i] - shapes[g][i]) ** 2;
          if (d < bd) {
            bd = d;
            glyph = g;
          }
        }
        // ---- vivid ink color (main pass port) ----
        const dev: [number, number, number] = [
          cellCol[0] - bg[0],
          cellCol[1] - bg[1],
          cellCol[2] - bg[2],
        ];
        const mag =
          Math.abs(dev[0]) * LW[0] + Math.abs(dev[1]) * LW[1] + Math.abs(dev[2]) * LW[2];
        const target = legibility * 0.75;
        const boost = clamp(target / Math.max(mag, 0.01), 1, 32);
        let vivid: [number, number, number] = [
          clamp(bg[0] + dev[0] * boost, 0, 1),
          clamp(bg[1] + dev[1] * boost, 0, 1),
          clamp(bg[2] + dev[2] * boost, 0, 1),
        ];
        const vividMag =
          Math.abs(vivid[0] - bg[0]) * LW[0] +
          Math.abs(vivid[1] - bg[1]) * LW[1] +
          Math.abs(vivid[2] - bg[2]) * LW[2];
        const inkLum = bg[0] * LW[0] + bg[1] * LW[1] + bg[2] * LW[2];
        const ink: [number, number, number] = inkLum < 0.5 ? [1, 1, 1] : [0.06, 0.06, 0.06];
        const mk = clamp((target - vividMag) / Math.max(target, 1e-3), 0, 1);
        vivid = [
          vivid[0] + (ink[0] - vivid[0]) * mk,
          vivid[1] + (ink[1] - vivid[1]) * mk,
          vivid[2] + (ink[2] - vivid[2]) * mk,
        ];
        const cellSig = clamp(mag * 1.6, 0, 1);
        const monoMix = 0.35 + (1.2 - 0.35) * cellSig;
        const col: [number, number, number] = [
          mono[0] * monoMix + (vivid[0] - mono[0] * monoMix) * colored,
          mono[1] * monoMix + (vivid[1] - mono[1] * monoMix) * colored,
          mono[2] * monoMix + (vivid[2] - mono[2] * monoMix) * colored,
        ];
        const finalCol: [number, number, number] = [
          clamp(bg[0] + (col[0] - bg[0]) * brightness, 0, 1),
          clamp(bg[1] + (col[1] - bg[1]) * brightness, 0, 1),
          clamp(bg[2] + (col[2] - bg[2]) * brightness, 0, 1),
        ];
        cellsInfo.push({ glyph, color: finalCol });
      }
    }

    // ---- main pass (ported): damped cursor, reveal circle, churn, tint ----
    const overlay = document.createElement("canvas");
    overlay.width = content.width;
    overlay.height = content.height;
    overlay.className = "decrypt-reveal-canvas";
    host.appendChild(overlay);
    const ctx = overlay.getContext("2d")!;
    ctx.scale(dpr, dpr);
    ctx.font = `600 ${Math.min(ch * 0.92, cw / 0.58)}px ${FONT_STACK}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";

    let raf = 0;
    let last = performance.now();
    let t = 0;
    const target = { x: 0, y: 0 };
    const cur = { x: 0, y: 0 };
    let wantActive = 0;
    let active = 0;
    let entered = false;
    let dismissed = false;
    let dismissTimer: ReturnType<typeof setTimeout> | undefined;
    const fadeCipher = () => {
      if (dismissed) return;
      dismissed = true;
      overlay.style.transition = "opacity 520ms ease-out";
      requestAnimationFrame(() => {
        overlay.style.opacity = "0";
      });
      dismissTimer = setTimeout(() => {
        cancelAnimationFrame(raf);
        host.removeEventListener("pointermove", onMove);
        host.removeEventListener("pointerleave", onLeave);
      }, 560);
    };
    const onMove = (e: PointerEvent) => {
      const rc = host.getBoundingClientRect();
      target.x = e.clientX - rc.left;
      target.y = e.clientY - rc.top;
      wantActive = 1;
      entered = true;
    };
    const onLeave = () => {
      wantActive = 0;
      if (!dismissOnLeave || !entered) return;
      fadeCipher();
    };
    host.addEventListener("pointermove", onMove);
    host.addEventListener("pointerleave", onLeave);

    const inner = radius * (1 - softness);
    const bandW = Math.max(radius * edgeWidth * 0.5, 6);
    const bandD0 = inner + (radius - inner) * 0.5;
    const veilAlpha = 1 - passthrough;
    const tau = Math.max(0.001, smoothing);
    const idleReroll = scramble * 0.35;
    const idleSpeed = Math.max(scrambleSpeed, 0.001);

    const tick = () => {
      const now = performance.now();
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      t += dt;
      const k = 1 - Math.exp(-dt / tau);
      cur.x += (target.x - cur.x) * k;
      cur.y += (target.y - cur.y) * k;
      active += (wantActive - active) * k;

      ctx.clearRect(0, 0, box.w, box.h);
      for (let r = 0; r < rows; r++) {
        for (let c = 0; c < cols; c++) {
          const info = cellsInfo[r * cols + c];
          const x = c * cw;
          const y = r * ch;
          const cx = x + cw / 2;
          const cy = y + ch / 2;
          const dist = Math.hypot(cx - cur.x, cy - cur.y);
          const e = (1 - smoothstep(inner, radius, dist)) * active;
          const bandD = dist - bandD0;
          const ring = Math.exp(-(bandD * bandD) / (2 * bandW * bandW)) * active;

          if (e >= 0.999 && ring <= 0.003) continue; // real content below shows

          // Encrypted base: background with passthrough of the content.
          ctx.globalAlpha = 1;
          ctx.fillStyle = `rgba(0,0,0,${(veilAlpha * (1 - e)).toFixed(4)})`;
          ctx.fillRect(x, y, cw + 0.5, ch + 0.5);

          if (info.glyph < 0) continue;
          // Glyph churn — ported hash cadence.
          const idx = r * cols + c;
          const rerollP = clamp(idleReroll + ring * edgeFlicker, 0, 1);
          const speed = idleSpeed * (1 + ring * 2.5);
          const ft = Math.floor(t * speed);
          const h =
            ghash(idx * 3.3 + 1.7, idx * 2.9 + 9.1) * 0.981 +
            ghash(ft * 0.717, ft * 0.523) * 0.019;
          let glyph = info.glyph;
          if (h < rerollP) {
            const pick = ghash(
              ((idx + 1) % 9973) * 0.103,
              ((idx + 1) % 9973) * 0.089 + ft * 0.717 + 3.7,
            );
            glyph = 1 + Math.floor(pick * (chars.length - 1));
          }
          // Edge tint + glow surge (ported color math).
          const base = info.color;
          const cellLum = base[0] * LW[0] + base[1] * LW[1] + base[2] * LW[2];
          const tm = ring * edgeTint;
          const glowK = 1 + ring * edgeGlow * 1.6;
          const mixR = mono[0] * Math.max(brightness, 1) * (0.6 + cellLum);
          const mixG = mono[1] * Math.max(brightness, 1) * (0.6 + cellLum);
          const mixB = mono[2] * Math.max(brightness, 1) * (0.6 + cellLum);
          const colR = clamp(bg[0] + (base[0] + (mixR - base[0]) * tm - bg[0]) * glowK, 0, 1);
          const colG = clamp(bg[1] + (base[1] + (mixG - base[1]) * tm - bg[1]) * glowK, 0, 1);
          const colB = clamp(bg[2] + (base[2] + (mixB - base[2]) * tm - bg[2]) * glowK, 0, 1);
          ctx.globalAlpha = 1 - e;
          ctx.fillStyle = `rgb(${Math.round(colR * 255)},${Math.round(colG * 255)},${Math.round(colB * 255)})`;
          ctx.fillText(chars[glyph], cx, cy);
        }
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(raf);
      if (dismissTimer !== undefined) clearTimeout(dismissTimer);
      host.removeEventListener("pointermove", onMove);
      host.removeEventListener("pointerleave", onLeave);
      overlay.remove();
    };
  }, [box, imgTick, cell, aspect, radius, softness, colored, color, brightness, legibility, contrast, exposure, scramble, scrambleSpeed, edgeWidth, edgeFlicker, edgeGlow, edgeTint, passthrough, threshold, background, smoothing, dismissOnLeave]);

  return (
    <div ref={hostRef} className={`decrypt-reveal ${className}`}>
      {children}
    </div>
  );
}

function step05(x: number) {
  return x >= 0.5 ? 1 : 0;
}
