import jsQR from "jsqr";

export const MAX_QR_PIXELS = 32_000_000;

/** Bounded image pyramid plus overlapping tiles retain small codes on large monitors. */
export function scanQrPixels(pixels: Uint8ClampedArray, width: number, height: number): string | null {
  if (!Number.isInteger(width) || !Number.isInteger(height) || width <= 0 || height <= 0 ||
      width * height > MAX_QR_PIXELS || pixels.length !== width * height * 4) throw new Error("size");
  function sample(x: number, y: number, w: number, h: number, scale: number) {
    const sw = Math.max(1, Math.floor(w * scale)), sh = Math.max(1, Math.floor(h * scale));
    const data = new Uint8ClampedArray(sw * sh * 4);
    for (let row = 0; row < sh; row++) for (let col = 0; col < sw; col++) {
      const source = ((y + Math.min(h - 1, Math.floor(row / scale))) * width + x + Math.min(w - 1, Math.floor(col / scale))) * 4;
      const target = (row * sw + col) * 4;
      // Composite transparent image pixels onto white before binarization.
      const alpha = pixels[source + 3] / 255;
      for (let c = 0; c < 3; c++) data[target + c] = pixels[source + c] * alpha + 255 * (1 - alpha);
      data[target + 3] = 255;
    }
    return jsQR(data, sw, sh, { inversionAttempts: "attemptBoth" })?.data || null;
  }
  for (const limit of [1200, 2400]) {
    const found = sample(0, 0, width, height, Math.min(1, limit / Math.max(width, height)));
    if (found) return found;
    if (Math.max(width, height) <= limit) return null;
  }
  for (let y = 0; y < height; y += 850) for (let x = 0; x < width; x += 850) {
    const found = sample(x, y, Math.min(1200, width - x), Math.min(1200, height - y), 1);
    if (found) return found;
  }
  return null;
}
