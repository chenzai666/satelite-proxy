// Particle sphere adapted from Canvas UI Particle Object:
// https://canvasui.dev/docs/components/particle-object
import { useEffect, useRef, useState } from "react";
import * as THREE from "three";

export type ParticleSphereState = "live" | "stopped" | "error" | "switching";

export interface ParticleSphereProps {
  className?: string;
  state?: ParticleSphereState;
  color?: string;
  count?: number;
  size?: number;
}

interface CloudSample {
  positions: Float32Array;
  colors: Float32Array;
  shades: Float32Array;
}

interface SphereOptions {
  count: number;
  size: number;
  sizeVariance: number;
  color: string;
  radius: number;
  strength: number;
  swirl: number;
  spring: number;
  damping: number;
  drift: number;
  scale: number;
  floatIntensity: number;
  rotationIntensity: number;
  floatSpeed: number;
  autoRotateSpeed: number;
  orbit: number;
  orbitSpeed: number;
}

interface SphereInstance {
  setOptions: (options: Partial<SphereOptions>) => void;
  destroy: () => void;
}

const VERT = `
in vec3 aColor;
in float aShade;
in float aSeed;
out vec3 vColor;
uniform float uTime;
uniform float uDrift;
uniform float uSize;
uniform float uVariance;
uniform float uDpr;
uniform float uRefDist;

void main() {
  vec3 p = position;
  float t = uTime + aSeed * 39.0;
  p += uDrift * 0.005 * vec3(
    sin(t * 1.7 + aSeed * 61.0),
    cos(t * 1.3 + aSeed * 23.0),
    sin(t * 2.3 + aSeed * 47.0));
  vec4 mv = modelViewMatrix * vec4(p, 1.0);
  float jitter = 1.0 + uVariance * (fract(aSeed * 7.13) - 0.5) * 1.4;
  gl_PointSize = clamp(
    uSize * uDpr * jitter * (uRefDist / max(-mv.z, 0.1)), 0.0, 64.0);
  vColor = aColor * aShade;
  gl_Position = projectionMatrix * mv;
}`;

const FRAG = `
precision highp float;
in vec3 vColor;
out vec4 outColor;

void main() {
  vec2 c = gl_PointCoord - 0.5;
  float r2 = dot(c, c);
  float alpha = 1.0 - smoothstep(0.16, 0.25, r2);
  if (alpha < 0.08) discard;
  outColor = vec4(vColor, alpha);
}`;

const CAMERA_DIR = new THREE.Vector3(0.15, 0.08, 1).normalize();
const CAMERA_DISTANCE = 4.05;
const LOGO_COLOR = "#eef1ff";

const GALAXY = [
  new THREE.Color("#1f8a72"),
  new THREE.Color("#2eb88a"),
  new THREE.Color("#55c89a"),
  new THREE.Color("#5ee0c0"),
  new THREE.Color("#6ef0dc"),
  new THREE.Color("#9af8e8"),
  new THREE.Color("#d8fff6"),
];

function galaxyColor(seed: number, x: number, y: number, z: number, out: THREE.Color) {
  const band = 1 - Math.min(Math.abs(y * 1.15 + x * 0.35) * 1.8, 1);
  const wander = (seed * 0.72 + (x + 0.5) * 0.18 + (z + 0.5) * 0.12) % 1;
  const idx = wander * (GALAXY.length - 1);
  const lo = Math.floor(idx);
  const hi = Math.min(lo + 1, GALAXY.length - 1);
  out.copy(GALAXY[lo]).lerp(GALAXY[hi], idx - lo);
  out.lerp(GALAXY[2], band * 0.28);
  if (seed > 0.92) {
    out.setRGB(0.85, 1, 0.95);
  }
}

function makeLogoTexture(): THREE.CanvasTexture {
  const size = 256;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  if (ctx) {
    ctx.clearRect(0, 0, size, size);
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.font = '600 168px "Segoe UI Symbol", "Apple Symbols", "Noto Sans Symbols", sans-serif';
    ctx.fillStyle = "rgba(255,255,255,0.28)";
    ctx.fillText("◈", size / 2, size / 2 + 6);
    ctx.fillStyle = "#ffffff";
    ctx.fillText("◈", size / 2, size / 2 + 6);
  }
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.needsUpdate = true;
  return texture;
}

function sampleSphere(count: number): CloudSample {
  const geometry = new THREE.IcosahedronGeometry(1, 4);
  const posAttr = geometry.getAttribute("position");
  const nrmAttr = geometry.getAttribute("normal");
  const index = geometry.getIndex();
  const triCount = Math.floor((index ? index.count : posAttr.count) / 3);

  const positions = new Float32Array(count * 3);
  const colors = new Float32Array(count * 3);
  const shades = new Float32Array(count);
  if (triCount === 0) {
    geometry.dispose();
    return { positions, colors, shades };
  }

  const a = new THREE.Vector3();
  const b = new THREE.Vector3();
  const c = new THREE.Vector3();
  const ab = new THREE.Vector3();
  const ac = new THREE.Vector3();
  const areas: number[] = [];
  let totalArea = 0;

  const vertexIndex = (tri: number, corner: number) =>
    index ? index.getX(tri * 3 + corner) : tri * 3 + corner;

  for (let tri = 0; tri < triCount; tri++) {
    a.fromBufferAttribute(posAttr, vertexIndex(tri, 0));
    b.fromBufferAttribute(posAttr, vertexIndex(tri, 1));
    c.fromBufferAttribute(posAttr, vertexIndex(tri, 2));
    ab.subVectors(b, a);
    ac.subVectors(c, a);
    totalArea += ab.cross(ac).length() * 0.5;
    areas.push(totalArea);
  }

  const normal = new THREE.Vector3();
  const light = new THREE.Vector3(0.5, 0.85, 0.55).normalize();
  const albedo = new THREE.Color();

  for (let i = 0; i < count; i++) {
    const pick = Math.random() * totalArea;
    let lo = 0;
    let hi = areas.length - 1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (areas[mid] < pick) lo = mid + 1;
      else hi = mid;
    }
    const tri = lo;
    const i0 = vertexIndex(tri, 0);
    const i1 = vertexIndex(tri, 1);
    const i2 = vertexIndex(tri, 2);

    let u = Math.random();
    let v = Math.random();
    if (u + v > 1) {
      u = 1 - u;
      v = 1 - v;
    }
    const w = 1 - u - v;

    a.fromBufferAttribute(posAttr, i0);
    b.fromBufferAttribute(posAttr, i1);
    c.fromBufferAttribute(posAttr, i2);
    a.multiplyScalar(w).addScaledVector(b, u).addScaledVector(c, v);
    const radius = i % 7 === 0 ? 0.62 + Math.random() * 0.3 : 1;
    a.multiplyScalar(radius * 0.5);

    positions[i * 3] = a.x;
    positions[i * 3 + 1] = a.y;
    positions[i * 3 + 2] = a.z;

    if (nrmAttr) {
      normal.set(
        nrmAttr.getX(i0) * w + nrmAttr.getX(i1) * u + nrmAttr.getX(i2) * v,
        nrmAttr.getY(i0) * w + nrmAttr.getY(i1) * u + nrmAttr.getY(i2) * v,
        nrmAttr.getZ(i0) * w + nrmAttr.getZ(i1) * u + nrmAttr.getZ(i2) * v,
      ).normalize();
    } else {
      normal.copy(a).normalize();
    }
    const shade = 0.42 + 0.68 * Math.max(normal.dot(light) * 0.5 + 0.5, 0);
    const lit = Math.min(Math.pow(shade, 1 / 2.2), 1);
    galaxyColor(Math.random(), a.x, a.y, a.z, albedo);
    colors[i * 3] = Math.min(albedo.r, 1);
    colors[i * 3 + 1] = Math.min(albedo.g, 1);
    colors[i * 3 + 2] = Math.min(albedo.b, 1);
    shades[i] = 0.62 + 0.38 * lit;
  }

  geometry.dispose();
  return { positions, colors, shades };
}

function createParticleSphere(
  canvas: HTMLCanvasElement,
  options: SphereOptions,
): SphereInstance | null {
  const config: SphereOptions = { ...options };

  let renderer: THREE.WebGLRenderer;
  try {
    renderer = new THREE.WebGLRenderer({
      canvas,
      antialias: false,
      alpha: true,
      powerPreference: "high-performance",
    });
  } catch {
    return null;
  }

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(52, 1, 0.1, 50);
  camera.position.copy(CAMERA_DIR).multiplyScalar(CAMERA_DISTANCE);
  camera.lookAt(0, 0, 0);

  const floatGroup = new THREE.Group();
  scene.add(floatGroup);
  const fitGroup = new THREE.Group();
  floatGroup.add(fitGroup);

  const material = new THREE.ShaderMaterial({
    glslVersion: THREE.GLSL3,
    vertexShader: VERT,
    fragmentShader: FRAG,
    transparent: true,
    depthWrite: true,
    uniforms: {
      uTime: { value: Math.random() * 100 },
      uDrift: { value: config.drift },
      uSize: { value: config.size },
      uVariance: { value: config.sizeVariance },
      uDpr: { value: 1 },
      uRefDist: { value: CAMERA_DISTANCE },
    },
  });

  const sample = sampleSphere(Math.max(Math.round(config.count), 16));
  const count = sample.positions.length / 3;
  const seeds = new Float32Array(count);
  const orbitRate = new Float32Array(count);
  for (let i = 0; i < count; i++) {
    const seed = Math.random();
    seeds[i] = seed;
    orbitRate[i] = 0.9 + seed * 0.2;
  }

  const geometry = new THREE.BufferGeometry();
  const positionAttr = new THREE.BufferAttribute(sample.positions.slice(), 3);
  positionAttr.setUsage(THREE.DynamicDrawUsage);
  geometry.setAttribute("position", positionAttr);
  geometry.setAttribute("aColor", new THREE.BufferAttribute(sample.colors, 3));
  geometry.setAttribute("aShade", new THREE.BufferAttribute(sample.shades, 1));
  geometry.setAttribute("aSeed", new THREE.BufferAttribute(seeds, 1));

  const homes = sample.positions;
  const velocities = new Float32Array(count * 3);
  const points = new THREE.Points(geometry, material);
  points.frustumCulled = false;
  points.renderOrder = 0;
  fitGroup.add(points);

  const logoMap = makeLogoTexture();
  const logoMat = new THREE.SpriteMaterial({
    map: logoMap,
    color: new THREE.Color(LOGO_COLOR),
    transparent: true,
    depthTest: true,
    depthWrite: false,
    sizeAttenuation: true,
  });
  const logo = new THREE.Sprite(logoMat);
  logo.scale.setScalar(0.33);
  logo.renderOrder = 1;
  logo.frustumCulled = false;
  fitGroup.add(logo);

  fitGroup.scale.setScalar(config.scale);

  const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
  let reducedMotion = motionQuery.matches;
  const onMotionChange = () => {
    reducedMotion = motionQuery.matches;
    if (reducedMotion) floatGroup.rotation.set(0, 0, 0);
    applyOptions();
  };
  motionQuery.addEventListener("change", onMotionChange);

  function applyOptions() {
    renderer.setClearColor(new THREE.Color("#000000"), 0);
    material.uniforms.uDrift.value = reducedMotion ? 0 : Math.max(config.drift, 0);
    material.uniforms.uSize.value = Math.max(config.size, 0.1);
    material.uniforms.uVariance.value = Math.min(
      Math.max(config.sizeVariance, 0),
      1,
    );
    if (!motionReady) {
      logoMat.opacity = motion.logoOpacity;
      fitGroup.scale.setScalar(sceneScale());
    }
  }

  const motion = {
    energy: config.orbit,
    orbitSpeed: config.orbitSpeed,
    spring: config.spring,
    damping: config.damping,
    swirl: config.swirl,
    drift: config.drift,
    floatSpeed: config.floatSpeed,
    autoRotateSpeed: config.autoRotateSpeed,
    floatIntensity: config.floatIntensity,
    rotationIntensity: config.rotationIntensity,
    logoOpacity: config.orbit > 0.2 ? 0.96 : 0.62,
  };
  let motionReady = false;
  let burst = 0;

  function easeMotion(delta: number) {
    const rising = config.orbit > motion.energy + 0.015;
    const rate = reducedMotion ? 18 : rising ? 3.5 : 2.4;
    const t = 1 - Math.exp(-rate * delta);
    motion.energy += (config.orbit - motion.energy) * t;
    motion.orbitSpeed += (config.orbitSpeed - motion.orbitSpeed) * t;
    motion.spring += (config.spring - motion.spring) * t;
    motion.damping += (config.damping - motion.damping) * t;
    motion.swirl += (config.swirl - motion.swirl) * t;
    motion.drift += (config.drift - motion.drift) * t;
    motion.floatSpeed += (config.floatSpeed - motion.floatSpeed) * t;
    motion.autoRotateSpeed += (config.autoRotateSpeed - motion.autoRotateSpeed) * t;
    motion.floatIntensity += (config.floatIntensity - motion.floatIntensity) * t;
    motion.rotationIntensity += (config.rotationIntensity - motion.rotationIntensity) * t;
    const logoTarget = motion.energy > 0.2 ? 0.96 : 0.62;
    motion.logoOpacity += (logoTarget - motion.logoOpacity) * t;
    logoMat.opacity = motion.logoOpacity;
    material.uniforms.uDrift.value = reducedMotion ? 0 : motion.drift;
    burst *= Math.exp(-(rising ? 2.1 : 3.6) * delta);
    fitGroup.scale.setScalar(sceneScale());
    motionReady = true;
  }

  const host = canvas.closest(".orbit") as HTMLElement | null;
  let viewRef = 168;

  function sceneScale() {
    const minSide = Math.min(
      Math.max(canvas.clientWidth, 1),
      Math.max(canvas.clientHeight, 1),
    );
    if (minSide < 8) return config.scale;
    return config.scale * (viewRef / minSide) * (1 + burst * 0.08);
  }

  function resize() {
    const width = Math.max(canvas.clientWidth, 1);
    const height = Math.max(canvas.clientHeight, 1);
    if (host) {
      viewRef = Math.max(Math.min(host.clientWidth, host.clientHeight), 1);
    }
    const pr = Math.min(window.devicePixelRatio || 1, 2);
    renderer.setPixelRatio(pr);
    renderer.setSize(width, height, false);
    material.uniforms.uDpr.value = pr;
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
    fitGroup.scale.setScalar(sceneScale());
  }

  const observer = new ResizeObserver(resize);
  observer.observe(canvas);
  resize();
  applyOptions();

  let pointerX = 0;
  let pointerY = 0;
  let pointerActive = false;
  let pointerSpeed = 0;
  let lastPointerX = 0;
  let lastPointerY = 0;
  let lastPointerTime = 0;
  let shoveX = 0;
  let shoveY = 0;

  function onPointerMove(event: PointerEvent) {
    const rect = canvas.getBoundingClientRect();
    pointerX = event.clientX - rect.left;
    pointerY = event.clientY - rect.top;
    const now = performance.now();
    if (pointerActive && lastPointerTime) {
      const dt = Math.max((now - lastPointerTime) / 1000, 1e-3);
      const dx = pointerX - lastPointerX;
      const dy = pointerY - lastPointerY;
      const speed = Math.hypot(dx, dy) / dt;
      pointerSpeed += (speed - pointerSpeed) * 0.35;
      if (speed > 1) {
        const inv = 1 / Math.max(Math.hypot(dx, dy), 1e-3);
        shoveX += (dx * inv - shoveX) * 0.4;
        shoveY += (dy * inv - shoveY) * 0.4;
      }
    }
    lastPointerX = pointerX;
    lastPointerY = pointerY;
    lastPointerTime = now;
    pointerActive = true;
  }

  function onPointerLeave() {
    pointerActive = false;
    pointerSpeed = 0;
    lastPointerTime = 0;
  }

  const pointerTarget = host ?? canvas;
  pointerTarget.addEventListener("pointermove", onPointerMove, { passive: true });
  pointerTarget.addEventListener("pointerleave", onPointerLeave, { passive: true });
  pointerTarget.addEventListener("pointercancel", onPointerLeave, { passive: true });

  const raycaster = new THREE.Raycaster();
  const ndc = new THREE.Vector2();
  const inverseMatrix = new THREE.Matrix4();
  const localOrigin = new THREE.Vector3();
  const localDir = new THREE.Vector3();
  const camRight = new THREE.Vector3();
  const camUp = new THREE.Vector3();
  const camBack = new THREE.Vector3();
  const localShove = new THREE.Vector3();

  interface Field {
    ox: number;
    oy: number;
    oz: number;
    dx: number;
    dy: number;
    dz: number;
    r: number;
    r2: number;
    pushAccel: number;
    shove: number;
    sx: number;
    sy: number;
    sz: number;
  }

  const fields: Field[] = [];

  function addField(
    cx: number,
    cy: number,
    speed: number,
    shx: number,
    shy: number,
    strength: number,
  ) {
    if (strength <= 0) return;
    const width = Math.max(canvas.clientWidth, 1);
    const height = Math.max(canvas.clientHeight, 1);
    ndc.set((cx / width) * 2 - 1, -(cy / height) * 2 + 1);
    raycaster.setFromCamera(ndc, camera);
    points.updateWorldMatrix(true, false);
    inverseMatrix.copy(points.matrixWorld).invert();
    localOrigin.copy(raycaster.ray.origin).applyMatrix4(inverseMatrix);
    localDir.copy(raycaster.ray.direction).transformDirection(inverseMatrix);

    const worldScale = Math.max(fitGroup.scale.x, 1e-4);
    const worldPerPx =
      (2 *
        camera.position.distanceTo(floatGroup.position) *
        Math.tan(THREE.MathUtils.degToRad(camera.fov) / 2)) /
      height;
    const localRadius = (Math.max(config.radius, 1) * worldPerPx) / worldScale;
    camera.matrixWorld.extractBasis(camRight, camUp, camBack);
    localShove
      .set(0, 0, 0)
      .addScaledVector(camRight, shx)
      .addScaledVector(camUp, -shy)
      .transformDirection(inverseMatrix);

    fields.push({
      ox: localOrigin.x,
      oy: localOrigin.y,
      oz: localOrigin.z,
      dx: localDir.x,
      dy: localDir.y,
      dz: localDir.z,
      r: localRadius,
      r2: localRadius * localRadius,
      pushAccel: 26 * strength,
      shove: Math.min(speed / 900, 2) * 14 * strength,
      sx: localShove.x,
      sy: localShove.y,
      sz: localShove.z,
    });
  }

  function kickOrbit() {
    const p = positionAttr.array as Float32Array;
    const kick = Math.max(config.orbitSpeed, 0.2) * 0.32;
    for (let i = 0; i < count; i++) {
      const ix = i * 3;
      const iz = ix + 2;
      velocities[ix] += -p[iz] * kick;
      velocities[iz] += p[ix] * kick;
    }
  }

  function simulate(delta: number) {
    const p = positionAttr.array as Float32Array;
    const h = homes;
    const v = velocities;

    const stiffness = 60 * Math.max(motion.spring, 0.05);
    const dampingRate = 3 + 12 * Math.min(Math.max(motion.damping, 0), 1);
    const decay = Math.exp(-dampingRate * delta);
    const swirl = Math.min(Math.max(motion.swirl, 0), 2);
    const orbit = reducedMotion ? 0 : Math.min(Math.max(motion.energy, 0), 1);
    const orbitBase = motion.orbitSpeed * (1 + burst * 0.28) * orbit;

    fields.length = 0;
    if (pointerActive && !reducedMotion && config.strength > 0) {
      addField(pointerX, pointerY, pointerSpeed, shoveX, shoveY, config.strength);
    }

    const fieldCount = fields.length;

    for (let i = 0; i < count; i++) {
      const ix = i * 3;
      const iy = ix + 1;
      const iz = ix + 2;
      let vx = v[ix];
      let vy = v[iy];
      let vz = v[iz];

      if (orbitBase > 0) {
        const da = orbitBase * delta;
        const extra = orbitBase * (orbitRate[i] - 1) * delta;
        const cs = Math.cos(da);
        const sn = Math.sin(da);
        const hx = h[ix];
        const hz = h[iz];
        h[ix] = hx * cs - hz * sn;
        h[iz] = hx * sn + hz * cs;
        const px = p[ix];
        const pz = p[iz];
        p[ix] = px * cs - pz * sn;
        p[iz] = px * sn + pz * cs;
        const rvx = vx * cs - vz * sn;
        const rvz = vx * sn + vz * cs;
        vx = rvx;
        vz = rvz;
        if (extra !== 0) {
          const ce = Math.cos(extra);
          const se = Math.sin(extra);
          const ehx = h[ix];
          const ehz = h[iz];
          h[ix] = ehx * ce - ehz * se;
          h[iz] = ehx * se + ehz * ce;
        }
      }

      for (let f = 0; f < fieldCount; f++) {
        const field = fields[f];
        const wx = p[ix] - field.ox;
        const wy = p[iy] - field.oy;
        const wz = p[iz] - field.oz;
        const t = Math.max(wx * field.dx + wy * field.dy + wz * field.dz, 0);
        let rx = wx - field.dx * t;
        let ry = wy - field.dy * t;
        let rz = wz - field.dz * t;
        const dist2 = rx * rx + ry * ry + rz * rz;
        if (dist2 >= field.r2) continue;
        const dist = Math.sqrt(dist2);
        const inv = 1 / Math.max(dist, 1e-5);
        rx *= inv;
        ry *= inv;
        rz *= inv;
        const fall = 1 - dist / field.r;
        const force = fall * fall * delta;
        const tx = field.dy * rz - field.dz * ry;
        const ty = field.dz * rx - field.dx * rz;
        const tz = field.dx * ry - field.dy * rx;
        vx += (rx + tx * swirl) * field.pushAccel * force + field.sx * field.shove * force;
        vy += (ry + ty * swirl) * field.pushAccel * force + field.sy * field.shove * force;
        vz += (rz + tz * swirl) * field.pushAccel * force + field.sz * field.shove * force;
      }

      if (orbit > 0) {
        const wx = p[ix];
        const wy = p[iy];
        const wz = p[iz];
        const pr = Math.hypot(wx, wy, wz);
        const inv = 1 / Math.max(pr, 1e-5);
        const nx = wx * inv;
        const ny = wy * inv;
        const nz = wz * inv;
        const homeR = Math.hypot(h[ix], h[iy], h[iz]);
        const pull = (homeR - pr) * (58 + 16 * orbit) * delta;
        vx += nx * pull;
        vy += ny * pull;
        vz += nz * pull;
      }

      const homeMix = orbit > 0 ? 0.55 : 1;
      vx += (h[ix] - p[ix]) * stiffness * homeMix * delta;
      vy += (h[iy] - p[iy]) * stiffness * homeMix * delta;
      vz += (h[iz] - p[iz]) * stiffness * homeMix * delta;
      vx *= decay;
      vy *= decay;
      vz *= decay;
      p[ix] += vx * delta;
      p[iy] += vy * delta;
      p[iz] += vz * delta;
      v[ix] = vx;
      v[iy] = vy;
      v[iz] = vz;
    }

    positionAttr.needsUpdate = true;
  }

  let inView = true;
  let loopRunning = false;
  let disposed = false;
  let lastTime = 0;
  let elapsed = Math.random() * 100;
  let spin = Math.random() * Math.PI * 2;

  function tick(time: number) {
    if (!inView) {
      lastTime = 0;
      stopLoop();
      return;
    }
    const delta = lastTime ? Math.min((time - lastTime) / 1000, 1 / 30) : 0;
    lastTime = time;

    if (delta > 0) easeMotion(delta);

    if (!reducedMotion) {
      elapsed += delta * motion.floatSpeed;
      spin += delta * motion.autoRotateSpeed * (0.18 + burst * 0.22);
      floatGroup.rotation.x =
        (Math.cos(elapsed / 4) / 8) * motion.rotationIntensity;
      floatGroup.rotation.y =
        spin + (Math.sin(elapsed / 4) / 8) * motion.rotationIntensity;
      floatGroup.rotation.z =
        (Math.sin(elapsed / 4) / 20) * motion.rotationIntensity;
      floatGroup.position.y = 0;
      logo.position.y = Math.sin(elapsed * 1.15) * 0.03;
      material.uniforms.uTime.value += delta;
    }

    pointerSpeed *= Math.exp(-3 * delta);
    if (delta > 0) simulate(delta);
    renderer.render(scene, camera);
  }

  function startLoop() {
    if (loopRunning || !inView || disposed) return;
    loopRunning = true;
    renderer.setAnimationLoop(tick);
  }

  function stopLoop() {
    if (!loopRunning) return;
    loopRunning = false;
    renderer.setAnimationLoop(null);
  }

  const viewObserver =
    typeof IntersectionObserver !== "undefined"
      ? new IntersectionObserver((entries) => {
          inView = entries[entries.length - 1]?.isIntersecting ?? true;
          if (inView) startLoop();
          else stopLoop();
        })
      : null;
  viewObserver?.observe(canvas);

  startLoop();
  if (config.orbit > 0.75 && !reducedMotion) kickOrbit();

  return {
    setOptions(next: Partial<SphereOptions>) {
      const wasOrbit = config.orbit;
      Object.assign(config, next);
      applyOptions();
      if (!reducedMotion && config.orbit > wasOrbit + 0.25) {
        burst = 1;
        kickOrbit();
      }
      startLoop();
    },
    destroy() {
      disposed = true;
      stopLoop();
      observer.disconnect();
      viewObserver?.disconnect();
      motionQuery.removeEventListener("change", onMotionChange);
      pointerTarget.removeEventListener("pointermove", onPointerMove);
      pointerTarget.removeEventListener("pointerleave", onPointerLeave);
      pointerTarget.removeEventListener("pointercancel", onPointerLeave);
      fitGroup.remove(points);
      fitGroup.remove(logo);
      geometry.dispose();
      material.dispose();
      logoMap.dispose();
      logoMat.dispose();
      renderer.dispose();
    },
  };
}

function optionsFor(
  state: ParticleSphereState,
  color: string,
  count: number,
  size: number,
): SphereOptions {
  const live = state === "live";
  const switching = state === "switching";
  const stopped = state === "stopped";
  return {
    count,
    size,
    sizeVariance: 0.72,
    color,
    radius: switching ? 130 : 100,
    strength: stopped ? 0.65 : 1,
    swirl: switching ? 1.05 : live ? 0.85 : 0.6,
    spring: live ? 0.45 : switching ? 0.62 : 1,
    damping: live ? 0.18 : switching ? 0.22 : 0.32,
    drift: stopped ? 0.22 : switching ? 0.7 : live ? 0.55 : 0.45,
    scale: 3.861,
    floatIntensity: stopped ? 0.45 : switching ? 1.25 : 1.05,
    rotationIntensity: stopped ? 0.35 : switching ? 0.95 : 0.8,
    floatSpeed: stopped ? 0.7 : switching ? 2.1 : live ? 1.5 : 1.1,
    autoRotateSpeed: stopped ? 0.35 : switching ? 2.2 : live ? 0.45 : 0.7,
    orbit: live ? 1 : switching ? 0.4 : 0,
    orbitSpeed: live ? 2.2 : switching ? 3.2 : 0,
  };
}

export function ParticleSphere({
  className,
  state = "stopped",
  color,
  count = 2400,
  size = 1.96,
}: ParticleSphereProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const instanceRef = useRef<SphereInstance | null>(null);
  const [failed, setFailed] = useState(false);
  const [initial] = useState(() => optionsFor(state, color ?? "", count, size));

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const instance = createParticleSphere(canvas, initial);
    if (!instance) {
      setFailed(true);
      return;
    }
    instanceRef.current = instance;
    return () => {
      instance.destroy();
      instanceRef.current = null;
    };
  }, [initial]);

  useEffect(() => {
    instanceRef.current?.setOptions(optionsFor(state, color ?? "", count, size));
  }, [state, color, count, size]);

  if (failed) {
    return <div className={`particle-sphere-fallback ${className ?? ""}`} aria-hidden />;
  }

  return (
    <div className={`particle-sphere ${className ?? ""}`} aria-hidden>
      <canvas ref={canvasRef} className="particle-sphere-canvas" />
    </div>
  );
}

export default ParticleSphere;
