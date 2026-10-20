import { scanQrPixels } from "./qrDecode";

self.onmessage = (event: MessageEvent<{ pixels: ArrayBuffer; width: number; height: number }>) => {
  try {
    const { pixels, width, height } = event.data;
    self.postMessage({ text: scanQrPixels(new Uint8ClampedArray(pixels), width, height) });
  } catch { self.postMessage({ error: true }); }
};
