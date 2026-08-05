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
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Tick so uptime label updates without waiting for status poll. */
  const [nowSec, setNowSec] = useState(() => Math.floor(Date.now() / 1000));

  const reload = useCallback(async () => {
    try {
      const [status, settings, nodes] = await Promise.all([
        getProxyStatus().catch(() => null),
        getSettings().catch(() => null),
        listAllNodes().catch(() => [] as ProxyNode[]),
      ]);
      setProxy(status);
      const id = settings?.current_node_id;
      setNode(id ? nodes.find((n) => n.id === id) ?? null : nodes[0] ?? null);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, []);

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

  useVisibleInterval(() => {
    void reload();
    setNowSec(Math.floor(Date.now() / 1000));
  }, 1000);

  const running = proxy?.running ?? false;
  const connecting =
    proxy?.core_state === "starting" || proxy?.core_state === "stopping";
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
    ? proxy?.core_state === "starting"
      ? "连接中…"
      : "停止中…"
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
            className={`orbit simple-orbit ${running ? "spin" : ""} ${connecting ? "pulse" : ""}`}
            aria-hidden
          >
            <div className="orbit-ring orbit-ring-a" />
            <div className="orbit-ring orbit-ring-b" />
            <div className="orbit-core">
              <span className="orbit-glyph simple-orbit-power" title="Power">
                ⏻
              </span>
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
        className="simple-card simple-node-card"
        onClick={() => onGoServers?.()}
      >
        <div className="simple-node-top">
          <span className="pill target-proxy">
            {node?.protocol?.toUpperCase() ?? "—"}
          </span>
        </div>
        <div className="simple-node-name">
          {node?.name ?? "未选择节点 · 点此管理"}
        </div>
        {running && node && (
          <div className="simple-node-latency-row">
            <span className="muted">节点延迟</span>
            {testing ? (
              <span className="lat lat-none mono">测速中…</span>
            ) : (
              <LatencyLabel ms={node.latency_ms} testedAt={node.latency_at} />
            )}
          </div>
        )}
        <div className="simple-card-hint muted">点击切换节点 / 订阅</div>
      </button>

      <div className="simple-card simple-kv-card">
        <div className="simple-kv-row">
          <span className="muted">本地代理</span>
          <span className="mono">
            {running ? `127.0.0.1:${port}` : "未运行"}
          </span>
        </div>
        <div className="simple-kv-row">
          <span className="muted">运行时长</span>
          <span className="mono">{uptimeLabel}</span>
        </div>
      </div>

      <div className="simple-card simple-traffic-card">
        <div className="simple-traffic-cell">
          <span className="muted">上传</span>
          <strong className="mono">{fmtSpeed(up)}</strong>
        </div>
        <div className="simple-traffic-cell">
          <span className="muted">下载</span>
          <strong className="mono">{fmtSpeed(down)}</strong>
        </div>
        <div className="simple-traffic-cell">
          <span className="muted">累计</span>
          <strong className="mono">{fmtBytes(total)}</strong>
        </div>
      </div>
    </div>
  );
}
