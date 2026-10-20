import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { captureQrScreen, readQrImage } from "../api";
import { qrImportPayload, shouldScanQrScreen, type QrImportPayload } from "../qrImport";
import { useI18n } from "../i18n";
import { GlassButton } from "./GlassButton";
import { RulesetMenu } from "./RulesetMenu";

async function decodeImage(dataUrl: string, signal: AbortSignal): Promise<string | null> {
  const match = /^data:(image\/(?:png|jpeg|webp|bmp|gif));base64,([A-Za-z0-9+/=]+)$/.exec(dataUrl);
  if (!match || match[2].length > 23_000_000 || signal.aborted) throw new Error("image");
  const bytes = Uint8Array.from(atob(match[2]), character => character.charCodeAt(0));
  const blob = new Blob([bytes], { type: match[1] });
  const bitmap = await createImageBitmap(blob);
  const { width, height } = bitmap;
  const canvas = document.createElement("canvas");
  let pixels: Uint8ClampedArray;
  try {
    if (width <= 0 || height <= 0 || width * height > 32_000_000) throw new Error("size");
    canvas.width = width; canvas.height = height;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (!context) throw new Error("canvas");
    context.drawImage(bitmap, 0, 0);
    pixels = context.getImageData(0, 0, width, height).data;
  } finally { bitmap.close(); canvas.width = canvas.height = 0; }
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL("../qrDecode.worker.ts", import.meta.url), { type: "module" });
    const clean = () => { worker.terminate(); clearTimeout(timer); signal.removeEventListener("abort", abort); };
    const abort = () => { clean(); reject(new Error("cancelled")); };
    const timer = setTimeout(() => { clean(); reject(new Error("timeout")); }, 30_000);
    signal.addEventListener("abort", abort, { once: true });
    if (signal.aborted) { abort(); return; }
    worker.onmessage = event => {
      clean();
      if (event.data.error) reject(new Error("decode")); else resolve(event.data.text);
    };
    worker.onerror = () => { clean(); reject(new Error("decode")); };
    worker.postMessage({ pixels: pixels.buffer, width, height }, [pixels.buffer]);
  });
}

export function QrImportButton({ disabled, onRecognized, onError }: {
  disabled?: boolean;
  onRecognized: (payload: QrImportPayload) => void;
  onError: (message: string) => void;
}) {
  const { t } = useI18n();
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const blocked = useRef(!!disabled);
  blocked.current = !!disabled;
  const abortRef = useRef<AbortController | null>(null);
  const close = useCallback(() => setAnchor(null), []);
  useEffect(() => () => abortRef.current?.abort(), []);
  const scan = useCallback(async (source: "screen" | "image") => {
    if (disabled || pending.current) return;
    pending.current = true;
    setBusy(true); close();
    const controller = new AbortController();
    abortRef.current = controller;
    try {
      let image: string;
      if (source === "image") {
        const path = await open({ multiple: false, filters: [{ name: "QR image", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "gif"] }] });
        if (typeof path !== "string" || controller.signal.aborted) return;
        image = await readQrImage(path);
      } else image = await captureQrScreen();
      if (controller.signal.aborted || blocked.current) return;
      const text = await decodeImage(image, controller.signal);
      if (controller.signal.aborted || blocked.current) return;
      if (!text) { onError(t("config.qrNotFound")); return; }
      const payload = qrImportPayload(text);
      if (!payload) { onError(t("config.qrUnsupported")); return; }
      onRecognized(payload);
    } catch (error) {
      if (!controller.signal.aborted) {
        onError(error instanceof Error ? t("config.qrFailed") : String(error));
      }
    } finally {
      pending.current = false;
      if (!controller.signal.aborted) setBusy(false);
    }
  }, [close, disabled, onError, onRecognized, t]);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target instanceof Element ? event.target : null;
      if (!shouldScanQrScreen(event, !!target?.closest("input,textarea,select,[contenteditable=true]"),
        !!disabled || pending.current || !!document.querySelector('[role="dialog"],.modal-backdrop'))) return;
      event.preventDefault(); void scan("screen");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [disabled, scan]);
  useEffect(() => {
    if (!anchor) return;
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Element && !anchor.contains(event.target) && !event.target.closest("[data-ruleset-menu]")) close();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") { close(); anchor.focus(); }
    };
    window.addEventListener("pointerdown", outside); window.addEventListener("keydown", key);
    return () => { window.removeEventListener("pointerdown", outside); window.removeEventListener("keydown", key); };
  }, [anchor, close]);
  return <>
    <GlassButton disabled={disabled || busy} aria-haspopup="menu" aria-expanded={!!anchor}
      onClick={event => setAnchor(anchor ? null : event.currentTarget)}>
      {t(busy ? "config.qrScanning" : "config.qrScan")}
    </GlassButton>
    {anchor && <RulesetMenu anchor={anchor} onClose={close}>
      <button type="button" className="rule-menu-item qr-scan-menu-item" role="menuitem" aria-keyshortcuts="Control+s" onClick={() => void scan("screen")}>
        <span>{t("config.qrScreen")}</span><kbd>Ctrl+S</kbd>
      </button>
      <button type="button" className="rule-menu-item" role="menuitem" onClick={() => void scan("image")}>
        {t("config.qrImage")}
      </button>
    </RulesetMenu>}
  </>;
}
