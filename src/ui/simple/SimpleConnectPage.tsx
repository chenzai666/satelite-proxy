import { useCallback, useEffect, useState } from "react";
import {
  getProxyStatus,
  getSettings,
  listAllNodes,
  startProxy,
  stopProxy,
  testNodesLatency,
} from "../../api";
import { useVisibleInterval } from "../../hooks/useVisibleInterval";
import type { ProxyNode, ProxyStatus } from "../../types";

function fmtSpeed(bps: number) {
  if (bps < 1024) return `${bps} B/s`;
  if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${(bps / (1024 * 1024)).toFixed(2)} MB/s`;
}

function fmtBytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/** Format elapsed seconds as H:MM:SS or M:SS. */
function fmtUptime(sec: number) {
  if (sec < 0 || !Number.isFinite(sec)) return "—";
  const s = Math.floor(sec);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(r).padStart(2, "0");
  if (h > 0) return `${h}:${mm}:${ss}`;
  return `${m}:${ss}`;
}

/** Latency colors: green <200 · yellow <300 · red ≥300 (same as Nodes page). */
function latencyClass(ms?: number | null) {
  if (ms == null || ms < 0) return "lat-none";
  if (ms < 200) return "lat-good";
  if (ms < 300) return "lat-ok";
  return "lat-slow";
}

function LatencyLabel({
  ms,
  testedAt,
}: {
  ms?: number | null;
  testedAt?: number | null;
}) {
  if (ms != null && ms >= 0) {
    return <span className={`lat mono ${latencyClass(ms)}`}>{ms} ms</span>;
  }
  if (testedAt != null) {
    return <span className="lat lat-timeout mono">timeout</span>;
  }
  return <span className="lat lat-none mono">未测</span>;
}

interface Props {
  onGoServers?: () => void;
}

export function SimpleConnectPage({ onGoServers }: Props) {
  const [proxy, setProxy] = useState<ProxyStatus | null>(null);
  const [node, setNode] = useState<ProxyNode | null>(null);
  const [nodeReady, setNodeReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Tick so uptime label updates without waiting for status poll. */
  const [nowSec, setNowSec] = useState(() => Math.floor(Date.now() / 1000));

  /** Wave 1: proxy status only (cheap; used for live traffic poll). */
  const reloadStatus = useCallback(async () => {
    try {
      const status = await getProxyStatus().catch(() => null);
      setProxy(status);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, []);

  /** Wave 2: settings + nodes (heavier; only on boot / after actions). */
  const reloadNode = useCallback(async () => {
    try {
      const [settings, nodes] = await Promise.all([
        getSettings().catch(() => null),
        listAllNodes().catch(() => [] as ProxyNode[]),
      ]);
      const id = settings?.current_node_id;
      setNode(id ? nodes.find((n) => n.id === id) ?? null : nodes[0] ?? null);
      setNodeReady(true);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      setNodeReady(true);
    }
  }, []);

  const reload = useCallback(async () => {
    // Status commits as soon as ready; node list fills in after (parallel fetch).
    const statusP = reloadStatus();
    const nodeP = reloadNode();
    await statusP;
    await nodeP;
  }, [reloadStatus, reloadNode]);

  /** Probe current node once; updates local node latency. */
  const probeCurrent = useCallback(async (nodeId: string) => {
    setTesting(true);
    setNode((prev) =>
      prev && prev.id === nodeId
        ? { ...prev, latency_ms: undefined, latency_at: undefined }
        : prev,
    );
    try {
      const batch = await testNodesLatency([nodeId], 3000);
      const r = batch.results.find((x) => x.id === nodeId);
      if (r) {
        setNode((prev) =>
          prev && prev.id === nodeId
            ? {
                ...prev,
                latency_ms: r.latency_ms ?? null,
                latency_at: r.tested_at,
              }
            : prev,
        );
      }
    } catch {
      // Keep prior / cleared state; user can re-start or open servers to retest.
    } finally {
      setTesting(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Poll status only (not full node list) — keeps uptime/speeds fresh cheaply.
  useVisibleInterval(() => {
    void reloadStatus();
    setNowSec(Math.floor(Date.now() / 1000));
  }, 1000);

  const running = proxy?.running ?? false;
  const connecting =
    busy ||
    proxy?.core_state === "starting" ||
    proxy?.core_state === "stopping";
  const orbitState = connecting
    ? "switching"
    : running
      ? "live"
      : proxy?.core_state === "error"
        ? "error"
        : "stopped";

  async function onToggle() {
    if (busy || connecting) return;
    setBusy(true);
    setError(null);
    try {
      if (running) {
        setProxy(await stopProxy());
        await reload();
      } else {
        const enableSys = proxy?.system_proxy ?? true;
        setProxy(await startProxy(enableSys));
        await reload();
        // After start, auto-probe current node once.
        const id = node?.id;
        if (id) void probeCurrent(id);
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  const statusText = connecting
    ? proxy?.core_state === "stopping" || (busy && running)
      ? "停止中…"
      : "连接中…"
    : running
      ? "已连接"
      : "未连接";

  const up = proxy?.upload_speed ?? 0;
  const down = proxy?.download_speed ?? 0;
  const total = (proxy?.upload_total ?? 0) + (proxy?.download_total ?? 0);
  const port = proxy?.mixed_port ?? 2080;
  // Backend-owned start time — survives UI destroy / dock reopen.
  const startedAt = proxy?.core_started_at ?? null;
  const uptimeLabel =
    running && startedAt != null && startedAt > 0
      ? fmtUptime(nowSec - startedAt)
      : "—";

  return (
    <div className="simple-page simple-connect">
      <div className={`simple-connect-hero is-${orbitState}`}>
        <button
          type="button"
          className="simple-orbit-btn"
          disabled={busy || connecting}
          onClick={() => void onToggle()}
          aria-label={running ? "断开连接" : "连接"}
          aria-pressed={running}
        >
          <div
            className={`orbit simple-orbit ${running || connecting ? "spin" : ""} ${connecting ? "pulse switching" : ""}`}
            aria-hidden
          >
            <div className="orbit-ring orbit-ring-a" />
            <div className="orbit-ring orbit-ring-b" />
            <div className="orbit-core">
              {connecting ? (
                <span className="lat-spinner orbit-core-spinner" aria-hidden />
              ) : (
                <span className="orbit-glyph simple-orbit-power" title="Power">
                  ⏻
                </span>
              )}
            </div>
            <div className="orbit-sat" />
          </div>
        </button>
        <div className={`simple-status-label ${running ? "on" : ""}`}>
          {statusText}
        </div>
      </div>

      {error && <div className="banner error simple-banner">{error}</div>}

      <button
        type="button"
        className="simple-glass-bar"
        onClick={() => onGoServers?.()}
      >
        <span className="simple-info-protocol">
          {!nodeReady ? (
            <span className="skel skel-inline skel-w-20" aria-hidden />
          ) : (
            (node?.protocol?.toUpperCase() ?? "—")
          )}
        </span>
        <span className="simple-info-name">
          {!nodeReady ? (
            <span className="skel skel-inline skel-w-50" aria-hidden />
          ) : (
            (node?.name ?? "未选择节点")
          )}
        </span>
        {running && node && (
          <span className="simple-info-latency">
            {testing ? (
              <span className="lat lat-none mono">测速中…</span>
            ) : (
              <LatencyLabel ms={node.latency_ms} testedAt={node.latency_at} />
            )}
          </span>
        )}
      </button>

      <div className="simple-glass-bar">
        <span className="muted mono">
          {running ? `Proxy on 127.0.0.1:${port}` : "未运行"}
        </span>
        <span className="muted mono">{uptimeLabel}</span>
      </div>

      <div className="simple-glass-bar simple-info-traffic">
        <span className="simple-info-traffic-cell">
          <span className="tr-dir up">↑</span>
          <strong className="mono">{fmtSpeed(up)}</strong>
        </span>
        <span className="simple-info-traffic-cell">
          <span className="tr-dir down">↓</span>
          <strong className="mono">{fmtSpeed(down)}</strong>
        </span>
        <span className="simple-info-traffic-cell">
          <span className="muted">总计</span>
          <strong className="mono">{fmtBytes(total)}</strong>
        </span>
      </div>
    </div>
  );
}
