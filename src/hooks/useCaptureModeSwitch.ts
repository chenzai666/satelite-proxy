import { useCallback, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { getProxyStatus, setCaptureMode } from "../api";
import type { ProxyStatus } from "../types";

export type CaptureMode = "off" | "system" | "tun";

export function resolveCaptureMode(p: ProxyStatus | null | undefined): CaptureMode {
  if (p?.capture_mode === "system" || p?.capture_mode === "tun") {
    return p.capture_mode;
  }
  if (p?.tun_enabled) return "tun";
  if (p?.system_proxy) return "system";
  return "off";
}

function optimisticProxy(
  prev: ProxyStatus | null | undefined,
  mode: CaptureMode,
): ProxyStatus | null {
  if (!prev) return null;
  return {
    ...prev,
    system_proxy: mode === "system",
    tun_enabled: mode === "tun",
    capture_mode: mode,
  };
}

/**
 * Optimistic capture-mode switch with a single-flight backend queue.
 * Rapid clicks only keep the latest target; the label spinner stays up until
 * that target is applied (or fails). Avoids concurrent set_capture_mode which
 * the core rejects with "内核正在切换".
 */
export function useCaptureModeSwitch(
  proxy: ProxyStatus | null,
  setProxy: (next: ProxyStatus | null | ((p: ProxyStatus | null) => ProxyStatus | null)) => void,
  onError: (msg: string) => void,
  onApplied?: (mode: CaptureMode, prevMode: CaptureMode) => void,
) {
  const [captureBusy, setCaptureBusy] = useState(false);
  const [captureUi, setCaptureUi] = useState<CaptureMode | null>(null);

  const desiredRef = useRef<CaptureMode | null>(null);
  const inFlightRef = useRef(false);
  /** Proxy snapshot before the current drain batch (for failure restore). */
  const baselineRef = useRef<ProxyStatus | null>(null);
  const baselineModeRef = useRef<CaptureMode>("off");

  const captureMode = captureUi ?? resolveCaptureMode(proxy);

  const drain = useCallback(async () => {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    setCaptureBusy(true);

    try {
      while (desiredRef.current != null) {
        const mode = desiredRef.current;
        desiredRef.current = null;
        try {
          const s = await setCaptureMode(mode);
          // Newer click arrived while we awaited — apply that next.
          if (desiredRef.current != null) continue;
          setProxy(s);
          setCaptureUi(null);
          onApplied?.(mode, baselineModeRef.current);
        } catch (e) {
          if (desiredRef.current != null) continue;
          const msg = typeof e === "string" ? e : String(e);
          onError(msg);
          setCaptureUi(null);
          const baseline = baselineRef.current;
          if (baseline) {
            setProxy(baseline);
          } else {
            const s = await getProxyStatus().catch(() => null);
            if (s) setProxy(s);
          }
        }
      }
    } finally {
      inFlightRef.current = false;
      if (desiredRef.current != null) {
        void drain();
      } else {
        setCaptureBusy(false);
      }
    }
  }, [onApplied, onError, setProxy]);

  const requestCaptureMode = useCallback(
    (mode: CaptureMode) => {
      if (mode === captureMode && !captureBusy) return;

      // First click in a batch: remember what to restore on total failure.
      if (!captureBusy && !inFlightRef.current) {
        baselineRef.current = proxy;
        baselineModeRef.current = resolveCaptureMode(proxy);
      }

      desiredRef.current = mode;

      // Paint target immediately (quick switch feel).
      flushSync(() => {
        setCaptureUi(mode);
        setCaptureBusy(true);
        const painted = optimisticProxy(proxy, mode);
        if (painted) setProxy(painted);
      });

      void drain();
    },
    [captureBusy, captureMode, drain, proxy, setProxy],
  );

  return {
    captureMode,
    captureBusy,
    requestCaptureMode,
  };
}
