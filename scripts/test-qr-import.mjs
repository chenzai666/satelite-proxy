import assert from "node:assert/strict";
import test from "node:test";
import QRCode from "qrcode";
import { qrImportPayload, shouldScanQrScreen } from "../src/qrImport.ts";
import { scanQrPixels } from "../src/qrDecode.ts";
import { generateNodeUuid } from "../src/nodeUuid.ts";

const link = "vless://11111111-1111-4111-8111-111111111111@example.test:443?security=tls#fixture";
function image(text, { width = 600, height = 600, x = 20, y = 20, scale = 4, inverted = false } = {}) {
  const qr = QRCode.create(text, { errorCorrectionLevel: "M" });
  const data = new Uint8ClampedArray(width * height * 4).fill(255);
  if (inverted) for (let i = 0; i < data.length; i += 4) data[i] = data[i + 1] = data[i + 2] = 0;
  for (let r = 0; r < qr.modules.size; r++) for (let c = 0; c < qr.modules.size; c++) {
    const color = qr.modules.get(r, c) ? (inverted ? 255 : 0) : (inverted ? 0 : 255);
    for (let dy = 0; dy < scale; dy++) for (let dx = 0; dx < scale; dx++) {
      const i = ((y + r * scale + dy) * width + x + c * scale + dx) * 4;
      data[i] = data[i + 1] = data[i + 2] = color;
    }
  }
  return { data, width, height };
}
test("真实二维码像素可识别，支持反色与大屏小码", () => {
  for (const options of [{}, { inverted: true }, { width: 3600, height: 800, x: 2400, y: 250, scale: 3 }]) {
    const { data, width, height } = image(link, options);
    assert.equal(scanQrPixels(data, width, height), link);
  }
});
test("空图、损坏尺寸和超限图不误识别", () => {
  assert.equal(scanQrPixels(new Uint8ClampedArray(100 * 100 * 4).fill(255), 100, 100), null);
  for (const [width, height] of [[0, 1], [1, -1], [NaN, 1], [10000, 10000], [2, 2]]) {
    assert.throws(() => scanQrPixels(new Uint8ClampedArray(4), width, height));
  }
});
test("识别仅填表：节点、订阅和一键订阅链接；拒绝任意内容与危险协议", () => {
  assert.deepEqual(qrImportPayload(link), { kind: "text", content: link });
  assert.deepEqual(qrImportPayload("https://example.test/sub?token=fixture"), { kind: "url", url: "https://example.test/sub?token=fixture", name: undefined });
  assert.deepEqual(qrImportPayload("clash://install-config?url=https%3A%2F%2Fexample.test%2Fsub&name=test"), { kind: "url", url: "https://example.test/sub", name: "test" });
  for (const text of ["", "arbitrary text", "javascript:alert(1)", "file:///C:/private", "https://user:password@example.test/", "clash://install-config?url=file%3A%2F%2F%2Fprivate", "x".repeat(300000)]) {
    assert.equal(qrImportPayload(text), null);
  }
});
test("Ctrl+S 不抢输入框、弹窗、重复和其他快捷键", () => {
  const key = { key: "s", ctrlKey: true, metaKey: false, altKey: false, shiftKey: false, repeat: false, defaultPrevented: false };
  assert.equal(shouldScanQrScreen(key, false, false), true);
  assert.equal(shouldScanQrScreen(key, true, false), false);
  assert.equal(shouldScanQrScreen(key, false, true), false);
  for (const patch of [{ altKey: true }, { shiftKey: true }, { repeat: true }, { defaultPrevented: true }, { ctrlKey: false }, { key: "c" }]) {
    assert.equal(shouldScanQrScreen({ ...key, ...patch }, false, false), false);
  }
});
test("UUID 符合 v4 格式并使用安全随机源", () => {
  const values = new Set(Array.from({ length: 1000 }, generateNodeUuid));
  assert.equal(values.size, 1000);
  for (const value of values) assert.match(value, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  const original = Object.getOwnPropertyDescriptor(globalThis, "crypto");
  let calls = 0;
  Object.defineProperty(globalThis, "crypto", { configurable: true, value: { getRandomValues(bytes) { calls++; bytes.fill(0); return bytes; } } });
  try { assert.equal(generateNodeUuid(), "00000000-0000-4000-8000-000000000000"); assert.equal(calls, 1); }
  finally { Object.defineProperty(globalThis, "crypto", original); }
});
