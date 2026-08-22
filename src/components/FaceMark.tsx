// Face ID smiley hero visual, ported from the interstellar Android app's
// FaceMark composable (Compose Canvas). The glyph is the exact Face ID SVG
// (corners + nose + eyes); the mouth is a programmatic bezier whose control
// points interpolate frown ↔ smile by "mood", driven by the proxy state:
// connected breathes, blinks and relaxes its smile; connecting pulses like a
// scan; stopped rests on a frown. Colors are interstellar's fixed iOS
// green/blue per light/dark theme — independent of the app accent preset.
import { useEffect, useRef } from "react";
import type { ParticleSphereState } from "./ParticleSphere";

export interface FaceMarkProps {
  state?: ParticleSphereState;
  className?: string;
}

interface FaceEngine {
  setState(next: ParticleSphereState): void;
  destroy(): void;
}

interface Rgba {
  r: number;
  g: number;
  b: number;
  a: number;
}

/** Corners + nose from face-id-svgrepo-com.svg (24×24 filled path). */
const FRAME_PATH = new Path2D(
  "M7.5,3 C7.77614237,3 8,3.22385763 8,3.5 C8,3.77614237 7.77614237,4 7.5,4 L5.5,4 C4.67157288,4 4,4.67157288 4,5.5 L4,7.53112887 C4,7.80727125 3.77614237,8.03112887 3.5,8.03112887 C3.22385763,8.03112887 3,7.80727125 3,7.53112887 L3,5.5 C3,4.11928813 4.11928813,3 5.5,3 L7.5,3 Z " +
    "M16.5,4 C16.2238576,4 16,3.77614237 16,3.5 C16,3.22385763 16.2238576,3 16.5,3 L18.5,3 C19.8807119,3 21,4.11928813 21,5.5 L21,7.5 C21,7.77614237 20.7761424,8 20.5,8 C20.2238576,8 20,7.77614237 20,7.5 L20,5.5 C20,4.67157288 19.3284271,4 18.5,4 L16.5,4 Z " +
    "M20,16.5 C20,16.2238576 20.2238576,16 20.5,16 C20.7761424,16 21,16.2238576 21,16.5 L21,18.5 C21,19.8807119 19.8807119,21 18.5,21 L16.5,21 C16.2238576,21 16,20.7761424 16,20.5 C16,20.2238576 16.2238576,20 16.5,20 L18.5,20 C19.3284271,20 20,19.3284271 20,18.5 L20,16.5 Z " +
    "M3,16.5 C3,16.2238576 3.22385763,16 3.5,16 C3.77614237,16 4,16.2238576 4,16.5 L4,18.5 C4,19.3284271 4.67157288,20 5.5,20 L7.5,20 C7.77614237,20 8,20.2238576 8,20.5 C8,20.7761424 7.77614237,21 7.5,21 L5.5,21 C4.11928813,21 3,19.8807119 3,18.5 L3,16.5 Z " +
    "M12,8.5 C12,8.22385763 12.2238576,8 12.5,8 C12.7761424,8 13,8.22385763 13,8.5 L13,12.5 C13,13.3284271 12.3284271,14 11.5,14 C11.2238576,14 11,13.7761424 11,13.5 C11,13.2238576 11.2238576,13 11.5,13 C11.7761424,13 12,12.7761424 12,12.5 L12,8.5 Z",
);

const LEFT_EYE_PATH = new Path2D(
  "M8,8.5 C8,8.22385763 8.22385763,8 8.5,8 C8.77614237,8 9,8.22385763 9,8.5 L9,9.5 C9,9.77614237 8.77614237,10 8.5,10 C8.22385763,10 8,9.77614237 8,9.5 L8,8.5 Z",
);

const RIGHT_EYE_PATH = new Path2D(
  "M16,8.5 C16,8.22385763 16.2238576,8 16.5,8 C16.7761424,8 17,8.22385763 17,8.5 L17,9.5 C17,9.77614237 16.7761424,10 16.5,10 C16.2238576,10 16,9.77614237 16,9.5 L16,8.5 Z",
);

/** Mood spring from the original: dampingRatio 0.84, stiffness 170. */
const MOOD_STIFFNESS = 170;
const MOOD_DAMPING = 2 * 0.84 * Math.sqrt(MOOD_STIFFNESS);
/** Color cross-fade and smile-relax ease durations (seconds). */
const COLOR_TWEEN = 0.36;
const RELAX_TWEEN = 0.42;
/** Idle loop cycle lengths (seconds, one direction of a Reverse repeat). */
const PULSE_PERIOD = 0.9;
const BREATHE_PERIOD = 2.4;
const BLINK_PERIOD = 4.4;
const RELAX_PERIOD = 6.4;

/** Fallback if the text CSS variable resolves to something unparseable. */
const FALLBACK_TEXT = "rgba(255, 255, 255, 0.92)";
/** Matches the dashboard error hue rgba(214, 139, 88). */
const ERROR_COLOR: Rgba = { r: 214, g: 139, b: 88, a: 1 };
/**
 * Interstellar palette — the face keeps the Android original's fixed iOS
 * system green/blue across states regardless of the app accent preset; only
 * the light/dark theme picks the shade.
 */
const FACE_COLORS = {
  aerospace: {
    live: { r: 0x30, g: 0xd1, b: 0x58, a: 1 }, // #30D158
    scan: { r: 0x0a, g: 0x84, b: 0xff, a: 1 }, // #0A84FF
  },
  day: {
    live: { r: 0x34, g: 0xc7, b: 0x59, a: 1 }, // #34C759
    scan: { r: 0x00, g: 0x7a, b: 0xff, a: 1 }, // #007AFF
  },
} as const;

/** Eyes with their Y-scale pivots (SVG units). */
const EYES: readonly { path: Path2D; px: number }[] = [
  { path: LEFT_EYE_PATH, px: 8.5 },
  { path: RIGHT_EYE_PATH, px: 16.5 },
];

const clamp01 = (t: number) => Math.min(Math.max(t, 0), 1);
const lerp = (from: number, to: number, t: number) => from + (to - from) * t;
const smootherstep = (t: number) => {
  const x = clamp01(t);
  return x * x * (3 - 2 * x);
};
/** 0 → 1 → 0 over x ∈ [0, 2] — RepeatMode.Reverse normalized shape. */
const triangle = (x: number) => 1 - Math.abs((x % 2) - 1);

/** Rest on the native smile → soften corners → hold → ease back. */
function relaxAt(t: number): number {
  if (t < 0.22) return 0;
  if (t < 0.4) return smootherstep((t - 0.22) / 0.18);
  if (t < 0.5) return 1;
  if (t < 0.76) return lerp(1, 0, smootherstep((t - 0.5) / 0.26));
  return 0;
}

/** Quick lid close/open around 68% of the blink cycle. */
function blinkAt(t: number): number {
  const start = 0.68;
  const dur = 0.085;
  const u = (t - start) / dur;
  if (u < 0 || u > 1) return 1;
  return u < 0.42
    ? lerp(1, 0.08, smootherstep(u / 0.42))
    : lerp(0.08, 1, smootherstep((u - 0.42) / 0.58));
}

/** SVG mouth. Relax drops the corners together so stroke width stays 1. */
function mouthPath(mood: number, relax: number): Path2D {
  const cx = 12;
  const halfW = 3.9;
  const dip = (px: number) => {
    const u = Math.min(((px - cx) / halfW) ** 2, 1);
    return relax * lerp(-0.04, 0.36, u);
  };
  const y = (px: number, py: number) => 16.05 + (py - 16.05) * mood + dip(px);
  const p = new Path2D();
  p.moveTo(8.1, y(8.1, 15.8));
  p.bezierCurveTo(
    7.93431458, y(7.93431458, 15.5790861),
    7.9790861, y(7.9790861, 15.2656854),
    8.2, y(8.2, 15.1),
  );
  p.bezierCurveTo(
    8.4209139, y(8.4209139, 14.9343146),
    8.73431458, y(8.73431458, 14.9790861),
    8.9, y(8.9, 15.2),
  );
  p.bezierCurveTo(
    9.81096778, y(9.81096778, 16.4146237),
    10.8353763, y(10.8353763, 17),
    12, y(12, 17),
  );
  p.bezierCurveTo(
    13.1646237, y(13.1646237, 17),
    14.1890322, y(14.1890322, 16.4146237),
    15.1, y(15.1, 15.2),
  );
  p.bezierCurveTo(
    15.2656854, y(15.2656854, 14.9790861),
    15.5790861, y(15.5790861, 14.9343146),
    15.8, y(15.8, 15.1),
  );
  p.bezierCurveTo(
    16.0209139, y(16.0209139, 15.2656854),
    16.0656854, y(16.0656854, 15.5790861),
    15.9, y(15.9, 15.8),
  );
  p.bezierCurveTo(
    14.8109678, y(14.8109678, 17.252043),
    13.502043, y(13.502043, 18),
    12, y(12, 18),
  );
  p.bezierCurveTo(
    10.497957, y(10.497957, 18),
    9.18903222, y(9.18903222, 17.252043),
    8.1, y(8.1, 15.8),
  );
  p.closePath();
  return p;
}

function targetMood(state: ParticleSphereState): number {
  switch (state) {
    case "live":
    case "switching":
      return 1;
    default:
      return -0.9;
  }
}

function parseColor(raw: string): Rgba | null {
  const s = raw.trim();
  let m = /^#([0-9a-f]{3})$/i.exec(s);
  if (m) {
    return {
      r: parseInt(m[1][0] + m[1][0], 16),
      g: parseInt(m[1][1] + m[1][1], 16),
      b: parseInt(m[1][2] + m[1][2], 16),
      a: 1,
    };
  }
  m = /^#([0-9a-f]{6})$/i.exec(s);
  if (m) {
    const n = parseInt(m[1], 16);
    return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255, a: 1 };
  }
  m = /^rgba?\(([^)]+)\)$/i.exec(s);
  if (m) {
    const parts = m[1].split(/[,/\s]+/).filter(Boolean);
    const r = Number.parseFloat(parts[0]);
    const g = Number.parseFloat(parts[1]);
    const b = Number.parseFloat(parts[2]);
    const a = parts[3] == null ? 1 : Number.parseFloat(parts[3]);
    if (![r, g, b, a].every(Number.isFinite)) return null;
    return { r, g, b, a };
  }
  return null;
}

function cssVar(name: string, fallback: string): Rgba {
  let raw = "";
  try {
    raw = getComputedStyle(document.documentElement).getPropertyValue(name);
  } catch {
    /* below fallback */
  }
  return parseColor(raw) ?? parseColor(fallback) ?? { r: 0, g: 0, b: 0, a: 1 };
}

function colorFor(state: ParticleSphereState): Rgba {
  if (state === "live" || state === "switching") {
    const palette =
      document.documentElement.dataset.theme === "day"
        ? FACE_COLORS.day
        : FACE_COLORS.aerospace;
    return state === "live" ? palette.live : palette.scan;
  }
  if (state === "error") return ERROR_COLOR;
  return cssVar("--text", FALLBACK_TEXT);
}

function rgbaCss(c: Rgba): string {
  return `rgba(${Math.round(c.r)}, ${Math.round(c.g)}, ${Math.round(c.b)}, ${clamp01(c.a)})`;
}

function createFaceEngine(canvas: HTMLCanvasElement): FaceEngine {
  const ctx = canvas.getContext("2d");
  let state: ParticleSphereState = "stopped";
  let disposed = false;
  let inView = true;
  let dpr = Math.min(window.devicePixelRatio || 1, 2);

  // Animation state — mood spring, color cross-fade, phase clock.
  let mood = targetMood(state);
  let moodVel = 0;
  let color = colorFor(state);
  let colorFrom = color;
  let colorTarget = color;
  let colorStart = -COLOR_TWEEN;
  let relaxActive = 0;
  let relaxFrom = 0;
  let relaxGoal = 0;
  let relaxStart = -RELAX_TWEEN;
  let phase = 0;
  let raf = 0;
  let last = 0;

  const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
  let reducedMotion = motionQuery.matches;

  function draw() {
    if (!ctx || disposed) return;
    const w = Math.max(canvas.clientWidth, 1);
    const h = Math.max(canvas.clientHeight, 1);
    const scanning = state === "switching";
    const connected = state === "live";
    const pulse = 1 + 0.045 * triangle(phase / PULSE_PERIOD);
    const breathe = 1 + 0.018 * triangle(phase / BREATHE_PERIOD);
    const blinkPhase = (phase / BLINK_PERIOD) % 1;
    const relaxPhase = (phase / RELAX_PERIOD) % 1;
    const relax = relaxActive * relaxAt(relaxPhase);
    const blink = connected && !reducedMotion ? blinkAt(blinkPhase) : 1;
    const liveMood = scanning
      ? mood - 0.1 + ((pulse - 1) / 0.045) * 0.22
      : mood;
    const faceScale = reducedMotion
      ? 1
      : scanning
        ? pulse
        : connected
          ? breathe
          : 1;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const s = Math.min(w, h) / 24;
    const dx = (w - 24 * s) / 2;
    const dy = (h - 24 * s) / 2;
    ctx.save();
    ctx.translate(w / 2, h / 2);
    ctx.scale(faceScale, faceScale);
    ctx.translate(-w / 2, -h / 2);
    ctx.translate(dx, dy);
    ctx.scale(s, s);
    ctx.fillStyle = rgbaCss(color);
    ctx.fill(FRAME_PATH);
    for (const { path, px } of EYES) {
      ctx.save();
      ctx.translate(px, 9);
      ctx.scale(1, blink);
      ctx.translate(-px, -9);
      ctx.fill(path);
      ctx.restore();
    }
    ctx.fill(mouthPath(liveMood, relax));
    ctx.restore();
  }

  function step(dt: number) {
    // Mood spring toward the state's target.
    const target = targetMood(state);
    moodVel += (-MOOD_STIFFNESS * (mood - target) - MOOD_DAMPING * moodVel) * dt;
    mood += moodVel * dt;
    if (Math.abs(mood - target) < 0.001 && Math.abs(moodVel) < 0.001) {
      mood = target;
      moodVel = 0;
    }
    // Smile-relax gate eases in only while connected.
    const relaxTarget = state === "live" ? 1 : 0;
    if (relaxTarget !== relaxGoal) {
      relaxGoal = relaxTarget;
      relaxFrom = relaxActive;
      relaxStart = phase;
    }
    const relaxT = smootherstep(clamp01((phase - relaxStart) / RELAX_TWEEN));
    relaxActive = lerp(relaxFrom, relaxTarget, relaxT);
    // Color cross-fade. Steady state pins to the target object instead of
    // allocating a fresh {r,g,b,a} every frame (60fps GC churn while idle).
    const t = clamp01((phase - colorStart) / COLOR_TWEEN);
    if (t >= 1) {
      if (color !== colorTarget) color = colorTarget;
    } else {
      const from = colorFrom;
      const k = smootherstep(t);
      color = {
        r: lerp(from.r, colorTarget.r, k),
        g: lerp(from.g, colorTarget.g, k),
        b: lerp(from.b, colorTarget.b, k),
        a: lerp(from.a, colorTarget.a, k),
      };
    }
  }

  function frame(now: number) {
    if (disposed) return;
    const dt = Math.min(Math.max((now - last) / 1000, 0), 0.05);
    last = now;
    phase += dt;
    step(dt);
    draw();
    raf = requestAnimationFrame(frame);
  }

  function startLoop() {
    if (raf || !inView || reducedMotion || disposed) return;
    last = performance.now();
    raf = requestAnimationFrame(frame);
  }

  function stopLoop() {
    if (!raf) return;
    cancelAnimationFrame(raf);
    raf = 0;
  }

  function syncColor(snap: boolean) {
    const next = colorFor(state);
    const unchanged =
      next.r === colorTarget.r &&
      next.g === colorTarget.g &&
      next.b === colorTarget.b &&
      next.a === colorTarget.a;
    if (snap || reducedMotion || unchanged) {
      colorFrom = next;
      color = next;
      colorTarget = next;
      colorStart = phase - COLOR_TWEEN;
    } else {
      colorFrom = color;
      colorTarget = next;
      colorStart = phase;
    }
  }

  function applyReducedMotion() {
    if (reducedMotion) {
      stopLoop();
      mood = targetMood(state);
      moodVel = 0;
      relaxActive = state === "live" ? 1 : 0;
      relaxFrom = relaxActive;
      relaxGoal = relaxActive;
      syncColor(true);
      draw();
    } else {
      startLoop();
    }
  }

  function resize() {
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = Math.max(canvas.clientWidth, 1);
    const h = Math.max(canvas.clientHeight, 1);
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    draw();
  }

  const resizeObserver = new ResizeObserver(resize);
  resizeObserver.observe(canvas);

  const viewObserver =
    typeof IntersectionObserver !== "undefined"
      ? new IntersectionObserver((entries) => {
          inView = entries[entries.length - 1]?.isIntersecting ?? true;
          if (inView) startLoop();
          else stopLoop();
        })
      : null;
  viewObserver?.observe(canvas);

  const onMotionChange = () => {
    reducedMotion = motionQuery.matches;
    applyReducedMotion();
  };
  motionQuery.addEventListener("change", onMotionChange);

  // Theme / accent presets are applied as inline CSS variables on <html>;
  // re-read the stroke colors whenever they change.
  const styleObserver = new MutationObserver(() => {
    if (!reducedMotion) syncColor(false);
    else {
      syncColor(true);
      draw();
    }
  });
  styleObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["style", "data-theme"],
  });

  resize();
  applyReducedMotion();

  return {
    setState(next: ParticleSphereState) {
      if (next === state) return;
      state = next;
      if (reducedMotion) {
        applyReducedMotion();
        return;
      }
      syncColor(false);
      startLoop();
    },
    destroy() {
      disposed = true;
      stopLoop();
      resizeObserver.disconnect();
      viewObserver?.disconnect();
      styleObserver.disconnect();
      motionQuery.removeEventListener("change", onMotionChange);
    },
  };
}

export function FaceMark({ state = "stopped", className }: FaceMarkProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const engineRef = useRef<FaceEngine | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const engine = createFaceEngine(canvas);
    engineRef.current = engine;
    return () => {
      engine.destroy();
      engineRef.current = null;
    };
  }, []);

  useEffect(() => {
    engineRef.current?.setState(state);
  }, [state]);

  return (
    <canvas
      ref={canvasRef}
      className={`face-mark ${className ?? ""}`}
      aria-hidden
    />
  );
}

export default FaceMark;
