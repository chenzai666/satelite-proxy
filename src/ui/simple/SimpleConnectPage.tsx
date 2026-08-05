import { useCallback, useEffect, useRef, useState } from "react";
import {
  getProxyStatus,
  getSettings,
  listAllNodes,
  startProxy,
  stopProxy,
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

/** Format elapsed ms as H:MM:SS or M:SS. */
function fmtUptime(ms: number) {
  if (ms < 0 || !Number.isFinite(ms)) return "—";
  const sec = Math.floor(ms / 1000);
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  if (h > 0) return `${h}:${mm}:${ss}`;
  return `${m}:${ss}`;
}

interface Props {
  onGoServers?: () => void;
}

export function SimpleConnectPage({ onGoServers }: Props) {
  const [proxy, setProxy] = useState<ProxyStatus | null>(null);
  const [node, setNode] = useState<ProxyNode | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Client-side clock when we first observed core running this session. */
  const [runningSince, setRunningSince] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const wasRunningRef = useRef(false);

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

      const isRun = !!status?.running;
      if (isRun && !wasRunningRef.current) {
        setRunningSince(Date.now());
      } else if (!isRun) {
        setRunningSince(null);
      }
      wasRunningRef.current = isRun;
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  useVisibleInterval(() => {
    void reload();
    setNow(Date.now());
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
        setRunningSince(null);
        wasRunningRef.current = false;
      } else {
        const enableSys = proxy?.system_proxy ?? true;
        setProxy(await startProxy(enableSys));
        setRunningSince(Date.now());
        wasRunningRef.current = true;
      }
      await reload();
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
  const uptimeLabel =
    running && runningSince != null ? fmtUptime(now - runningSince) : "—";

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
          {node?.latency_ms != null && node.latency_ms >= 0 ? (
            <span className="lat lat-good mono">{node.latency_ms}ms</span>
          ) : (
            <span className="muted mono">—</span>
          )}
        </div>
        <div className="simple-node-name">
          {node?.name ?? "未选择节点 · 点此管理"}
        </div>
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
