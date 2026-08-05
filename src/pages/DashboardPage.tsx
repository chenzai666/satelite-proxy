import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getCoreInfo,
  getProxyStatus,
  getSettings,
  listAllNodes,
  listSubscriptions,
  previewSingboxConfig,
  restartProxy,
  setOutboundMode,
  setSystemProxy,
  setTunEnabled,
  startProxy,
  smartSwitchNow,
  stopProxy,
  updateSettings,
} from "../api";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import { useI18n } from "../i18n";
import type {
  GenerateConfigResult,
  OutboundMode,
  ProxyNode,
  ProxyStatus,
  SubscriptionView,
} from "../types";

interface Props {
  onGoProfiles?: () => void;
  onGoNodes?: () => void;
  onGoTraffic?: () => void;
  onGoSettings?: () => void;
}

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

function fmtLatency(ms?: number | null) {
  if (ms == null || ms < 0) return "—";
  return `${ms} ms`;
}

function relativeAgo(
  ts: number,
  t: (k: "common.justNow" | "common.minutesAgo" | "common.hoursAgo" | "common.daysAgo", v?: Record<string, string | number>) => string,
) {
  if (!ts) return "—";
  const sec = Math.max(0, Math.floor(Date.now() / 1000 - ts));
  if (sec < 60) return t("common.justNow");
  if (sec < 3600) return t("common.minutesAgo", { n: Math.floor(sec / 60) });
  if (sec < 86400) return t("common.hoursAgo", { n: Math.floor(sec / 3600) });
  return t("common.daysAgo", { n: Math.floor(sec / 86400) });
}

export function DashboardPage({
  onGoProfiles,
  onGoNodes,
  onGoTraffic,
  onGoSettings,
}: Props) {
  const { t } = useI18n();
  const [subs, setSubs] = useState<SubscriptionView[]>([]);
  const [nodes, setNodes] = useState<ProxyNode[]>([]);
  const [currentNode, setCurrentNode] = useState<ProxyNode | null>(null);
  const [settingsPorts, setSettingsPorts] = useState({ mixed: 2080, api: 19090 });
  const [coreLabel, setCoreLabel] = useState("—");
  const [coreVersion, setCoreVersion] = useState<string | null>(null);
  const [proxy, setProxy] = useState<ProxyStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<GenerateConfigResult | null>(null);
  const [showPreview, setShowPreview] = useState(false);
  const [sysProxyBusy, setSysProxyBusy] = useState(false);
  const [tunBusy, setTunBusy] = useState(false);
  /** Bootstrap probe after enabling smart switch (does not lock other controls). */
  const [smartProbing, setSmartProbing] = useState(false);
  const smartGenRef = useRef(0);
  const [modeBusy, setModeBusy] = useState(false);
  const [envCopied, setEnvCopied] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);

  const reload = useCallback(async () => {
    setError(null);
    try {
      const [subList, nodeList, settings, core, status] = await Promise.all([
        listSubscriptions(),
        listAllNodes(),
        getSettings(),
        getCoreInfo().catch(() => null),
        getProxyStatus().catch(() => null),
      ]);
      setSubs(subList);
      setNodes(nodeList);
      setSettingsPorts({ mixed: settings.mixed_port, api: settings.api_port });
      setProxy(status);
      const cur =
        nodeList.find((n) => n.id === settings.current_node_id) ??
        nodeList[0] ??
        null;
      setCurrentNode(cur);
      if (core?.installed) {
        const ver = (core.version ?? "ok").replace(/^v/, "");
        const tag =
          core.source === "bundled"
            ? t("settings.coreBundled")
            : core.source === "downloaded"
              ? t("settings.coreUser")
              : "";
        setCoreVersion(ver);
        setCoreLabel(tag ? `${ver} · ${tag}` : ver);
      } else {
        setCoreVersion(null);
        setCoreLabel(t("settings.coreMissing"));
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, [t]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useVisibleInterval(() => {
    void getProxyStatus()
      .then(setProxy)
      .catch(() => undefined);
  }, 2000);

  useEffect(() => {
    if (!moreOpen) return;
    function onDoc(e: MouseEvent) {
      if (moreRef.current && !moreRef.current.contains(e.target as Node)) {
        setMoreOpen(false);
      }
    }
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [moreOpen]);

  async function onStart() {
    setBusy(true);
    setError(null);
    try {
      const s = await startProxy(false);
      setProxy(s);
      await reload();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onToggleSystemProxy(next: boolean) {
    setSysProxyBusy(true);
    setError(null);
    try {
      const s = await setSystemProxy(next);
      setProxy(s);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      const s = await getProxyStatus().catch(() => null);
      if (s) setProxy(s);
    } finally {
      setSysProxyBusy(false);
    }
  }

  async function onToggleTun(next: boolean) {
    setTunBusy(true);
    setError(null);
    try {
      const s = await setTunEnabled(next);
      setProxy(s);
      await reload();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      const s = await getProxyStatus().catch(() => null);
      if (s) setProxy(s);
    } finally {
      setTunBusy(false);
    }
  }

  async function onToggleSmartSwitch(next: boolean) {
    setError(null);

    // Turn off: invalidate any in-flight bootstrap probe and free the UI immediately.
    if (!next) {
      smartGenRef.current += 1;
      setSmartProbing(false);
      setProxy((prev) => (prev ? { ...prev, smart_switch: false } : prev));
      try {
        await updateSettings({ smartSwitch: false });
        const s = await getProxyStatus().catch(() => null);
        if (s) setProxy(s);
      } catch (e) {
        setError(typeof e === "string" ? e : String(e));
      }
      return;
    }

    // Turn on: enable setting, then probe without locking other quick controls.
    const gen = ++smartGenRef.current;
    setSmartProbing(true);
    setProxy((prev) => (prev ? { ...prev, smart_switch: true } : prev));
    try {
      await updateSettings({ smartSwitch: true });
      if (gen !== smartGenRef.current) {
        // User turned off while enabling; re-assert off in case our write won the race.
        await updateSettings({ smartSwitch: false }).catch(() => {});
        return;
      }
      try {
        const r = await smartSwitchNow();
        if (gen !== smartGenRef.current) return;
        if (r.message === "core not running") {
          setError(t("dashboard.smartSwitchNeedCore"));
        } else if (r.message === "all probes failed") {
          setError(t("dashboard.smartSwitchProbeFail"));
        } else if (r.message === "no nodes") {
          setError(t("dashboard.smartSwitchNoNodes"));
        } else if (r.message === "clash api unavailable") {
          setError(t("dashboard.smartSwitchProbeFail"));
        }
      } catch (probeErr) {
        if (gen !== smartGenRef.current) return;
        setError(
          typeof probeErr === "string" ? probeErr : String(probeErr),
        );
      }
      if (gen !== smartGenRef.current) return;
      try {
        await reload();
        if (gen !== smartGenRef.current) return;
        setProxy((prev) =>
          prev ? { ...prev, smart_switch: true } : prev,
        );
      } catch {
        /* ignore */
      }
    } catch (e) {
      if (gen === smartGenRef.current) {
        setError(typeof e === "string" ? e : String(e));
        setProxy((prev) =>
          prev ? { ...prev, smart_switch: false } : prev,
        );
      }
    } finally {
      if (gen === smartGenRef.current) setSmartProbing(false);
    }
  }

  async function onSetMode(mode: OutboundMode) {
    if ((proxy?.outbound_mode ?? "rule") === mode || modeBusy) return;
    setModeBusy(true);
    setError(null);
    try {
      const s = await setOutboundMode(mode);
      setProxy(s);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      const s = await getProxyStatus().catch(() => null);
      if (s) setProxy(s);
    } finally {
      setModeBusy(false);
    }
  }

  async function onStop() {
    setBusy(true);
    setError(null);
    try {
      const s = await stopProxy();
      setProxy(s);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onRestart() {
    setBusy(true);
    setError(null);
    setMoreOpen(false);
    try {
      const s = await restartProxy();
      setProxy(s);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onPreview() {
    setBusy(true);
    setError(null);
    setMoreOpen(false);
    try {
      const r = await previewSingboxConfig();
      setResult(r);
      setShowPreview(true);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  const running = proxy?.running ?? false;
  const stateLabel = proxy?.core_state ?? "stopped";
  const outboundMode = (proxy?.outbound_mode ?? "rule") as OutboundMode;
  // Smart bootstrap probe must not lock routing / sys proxy / TUN.
  const controlsBusy = busy || sysProxyBusy || tunBusy || modeBusy;
  const smartSwitchOn = proxy?.smart_switch ?? false;
  const nodeCount = nodes.length;
  const subCount = subs.length;
  const mixedPort = proxy?.mixed_port ?? settingsPorts.mixed;

  const switching =
    stateLabel === "starting" || stateLabel === "stopping" || busy;
  const isError = stateLabel === "error" || (!!proxy?.error && !running);

  const stateUpper = running
    ? "RUNNING"
    : switching
      ? stateLabel === "stopping"
        ? "STOPPING"
        : "STARTING"
      : isError
        ? "ERROR"
        : "STOPPED";

  const dotClass = running
    ? "on"
    : switching
      ? "busy"
      : isError
        ? "off"
        : "off";

  const orbitState = running
    ? "live"
    : switching
      ? "switching"
      : isError
        ? "error"
        : "stopped";

  const heroTitle = running
    ? currentNode?.name ?? t("dashboard.disconnected")
    : isError
      ? t("dashboard.errorTitle")
      : t("dashboard.disconnected");

  const heroSub = running
    ? [currentNode?.protocol?.toUpperCase(), fmtLatency(currentNode?.latency_ms)]
        .filter(Boolean)
        .join(" · ")
    : t("dashboard.desc");

  /** Best / avg among nodes that have a successful latency sample. */
  const latencyStats = useMemo(() => {
    const samples: number[] = nodes
      .map((n) => n.latency_ms)
      .filter((ms): ms is number => ms != null && ms >= 0);
    if (samples.length === 0) {
      return { best: null as number | null, avg: null as number | null, n: 0 };
    }
    const best = Math.min(...samples);
    const avg = Math.round(samples.reduce((a, b) => a + b, 0) / samples.length);
    return { best, avg, n: samples.length };
  }, [nodes]);

  async function onCopyEnv() {
    const text = `export all_proxy=http://127.0.0.1:${mixedPort}`;
    try {
      await navigator.clipboard.writeText(text);
      setEnvCopied(true);
      setMoreOpen(false);
      window.setTimeout(() => setEnvCopied(false), 1500);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }

  const activeSub = useMemo(() => {
    const enabled = subs.filter((s) => s.enabled);
    return enabled[0] ?? subs[0] ?? null;
  }, [subs]);

  const subQuotaLabel = useMemo(() => {
    const tr = activeSub?.traffic;
    if (!tr) return "—";
    const used = (tr.upload ?? 0) + (tr.download ?? 0);
    if (tr.total && tr.total > 0) {
      const pct = Math.min(100, Math.round((used / tr.total) * 100));
      return `${pct}% · ${fmtBytes(used)} / ${fmtBytes(tr.total)}`;
    }
    if (tr.quota_remaining != null) {
      return t("common.remaining", { n: fmtBytes(tr.quota_remaining) });
    }
    if (used > 0) return fmtBytes(used);
    return "—";
  }, [activeSub, t]);

  const modeLabel =
    outboundMode === "rule"
      ? t("dashboard.modeRule")
      : outboundMode === "global"
        ? t("dashboard.modeGlobal")
        : t("dashboard.modeDirect");

  return (
    <div className="page dashboard-page">
      {error && <div className="banner error">{error}</div>}
      {proxy?.error && !running && (
        <div className="banner error">core: {proxy.error}</div>
      )}

      {/* —— Hero: orbit + status + embedded controls (no floating QC card) —— */}
      <section className={`dash-hero is-${orbitState}`}>
        <div
          className={`orbit ${running ? "spin" : ""} ${switching ? "pulse" : ""}`}
          aria-hidden
        >
          <div className="orbit-ring orbit-ring-a" />
          <div className="orbit-ring orbit-ring-b" />
          <div className="orbit-core">
            <span className="orbit-glyph">◈</span>
          </div>
          <div className="orbit-sat" />
        </div>

        <div className="dash-hero-copy">
          <div className="dash-kicker mono">
            <span className={`status-dot ${dotClass}`} />
            {stateUpper}
            <span className="dash-kicker-sep">·</span>
            SING-BOX {coreVersion ?? coreLabel}
          </div>

          <h1 className="dash-hero-title">{heroTitle}</h1>
          <p className="dash-hero-desc">{heroSub}</p>

          <div className="dash-hero-actions">
            {!running ? (
              <button
                type="button"
                className="btn-pill"
                disabled={busy || nodeCount === 0 || switching}
                onClick={() => void onStart()}
              >
                {busy || stateLabel === "starting"
                  ? t("dashboard.starting")
                  : isError
                    ? t("dashboard.retry")
                    : t("dashboard.start")}
              </button>
            ) : (
              <button
                type="button"
                className="btn-pill danger"
                disabled={busy || switching}
                onClick={() => void onStop()}
              >
                {t("dashboard.stop")}
              </button>
            )}

            <button
              type="button"
              className="btn-pill secondary"
              disabled={nodeCount === 0}
              onClick={() => onGoNodes?.()}
            >
              {t("dashboard.switchNode")}
            </button>

            <div className="dash-more" ref={moreRef}>
              <button
                type="button"
                className="btn-pill ghost dash-more-btn"
                aria-expanded={moreOpen}
                aria-haspopup="menu"
                onClick={() => setMoreOpen((v) => !v)}
              >
                ···
              </button>
              {moreOpen && (
                <div className="dash-more-menu card glass" role="menu">
                  <button
                    type="button"
                    role="menuitem"
                    disabled={busy || !running}
                    onClick={() => void onRestart()}
                  >
                    {t("dashboard.restart")}
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => void onCopyEnv()}
                  >
                    {envCopied
                      ? t("dashboard.envCopied")
                      : t("dashboard.copyEnv")}
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    disabled={busy || nodeCount === 0}
                    onClick={() => void onPreview()}
                  >
                    {t("common.preview")}
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setMoreOpen(false);
                      onGoSettings?.();
                    }}
                  >
                    {t("dashboard.advancedSettings")}
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Right rail: light controls, no card chrome */}
        <aside className="dash-side-rail" aria-label="Quick controls">
          <div className="dash-rail-title mono">{t("dashboard.quickControls")}</div>
          <div className="dash-inline-row dash-rail-block">
            <span className="dash-inline-label">{t("dashboard.routing")}</span>
            <div
              className="segmented compact mode-seg dash-inline-seg"
              role="group"
              aria-label={t("dashboard.routing")}
            >
              {/* Sliding indicator: width = 1/3 of the track, translateX follows active index. */}
              <span
                className="seg-indicator"
                aria-hidden="true"
                style={{
                  transform: `translateX(${
                    outboundMode === "rule" ? 0 : outboundMode === "global" ? 100 : 200
                  }%)`,
                }}
              />
              {(
                [
                  ["rule", t("dashboard.modeRule")],
                  ["global", t("dashboard.modeGlobal")],
                  ["direct", t("dashboard.modeDirect")],
                ] as const
              ).map(([key, label]) => (
                <button
                  key={key}
                  type="button"
                  className={`seg ${outboundMode === key ? "active" : ""}`}
                  disabled={controlsBusy}
                  onClick={() => void onSetMode(key)}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>
          <div className="dash-inline-row dash-inline-switch">
            <span className="dash-inline-label">
              {t("dashboard.sysProxyTitle")}
            </span>
            <button
              type="button"
              role="switch"
              aria-checked={proxy?.system_proxy ?? false}
              aria-label={t("dashboard.sysProxyTitle")}
              className={`switch small ${proxy?.system_proxy ? "on" : ""}`}
              disabled={controlsBusy}
              onClick={() =>
                void onToggleSystemProxy(!(proxy?.system_proxy ?? false))
              }
            >
              <span className="switch-thumb" />
            </button>
          </div>
          <div className="dash-inline-row dash-inline-switch">
            <span className="dash-inline-label">{t("dashboard.tunTitle")}</span>
            <button
              type="button"
              role="switch"
              aria-checked={proxy?.tun_enabled ?? false}
              aria-label={t("dashboard.tunTitle")}
              className={`switch small ${proxy?.tun_enabled ? "on" : ""}`}
              disabled={controlsBusy || nodeCount === 0}
              onClick={() => void onToggleTun(!(proxy?.tun_enabled ?? false))}
            >
              <span className="switch-thumb" />
            </button>
          </div>
          <div className="dash-inline-row dash-inline-switch">
            <span
              className={`dash-inline-label${smartProbing ? " dash-smart-probing" : ""}`}
              title={
                smartProbing
                  ? t("dashboard.smartSwitchProbing")
                  : t("dashboard.smartSwitchDesc")
              }
            >
              {smartProbing ? (
                <>
                  <span className="lat-spinner dash-smart-spinner" aria-hidden />
                  <span>{t("dashboard.smartSwitchProbing")}</span>
                </>
              ) : (
                t("dashboard.smartSwitch")
              )}
            </span>
            <button
              type="button"
              role="switch"
              aria-checked={smartSwitchOn}
              aria-busy={smartProbing}
              aria-label={
                smartProbing
                  ? t("dashboard.smartSwitchProbing")
                  : t("dashboard.smartSwitch")
              }
              title={
                smartProbing
                  ? t("dashboard.smartSwitchProbingHint")
                  : t("dashboard.smartSwitchDesc")
              }
              className={`switch small ${smartSwitchOn ? "on" : ""}`}
              disabled={nodeCount === 0 && !smartSwitchOn}
              onClick={() => void onToggleSmartSwitch(!smartSwitchOn)}
            >
              <span className="switch-thumb" />
            </button>
          </div>
        </aside>
      </section>

      {subCount === 0 && (
        <div className="dashboard-setup card glass">
          <p className="dashboard-setup-hint muted">
            {t("dashboard.noProfileHint")}
          </p>
          <button
            type="button"
            className="btn-pill"
            onClick={() => onGoProfiles?.()}
          >
            {t("dashboard.goAddProfile")}
          </button>
        </div>
      )}

      {/* —— 6 cards: core / traffic / quality · conns / sub / system —— */}
      <section className="instrument-grid instrument-grid-6" aria-label="Telemetry">
        <article className="instrument accent-green">
          <header className="instrument-head">
            <span className="instrument-label">{t("dashboard.cardCore")}</span>
            <span className={`instrument-tag ${running ? "ok" : ""}`}>
              {running ? "ONLINE" : switching ? "…" : "IDLE"}
            </span>
          </header>
          <div className="instrument-value sm">
            {running
              ? t("dashboard.coreRunning")
              : isError
                ? t("dashboard.coreError")
                : t("dashboard.coreStopped")}
          </div>
          <div className="instrument-kv mono">
            <div>
              <span className="kv-k">{t("dashboard.version")}</span>
              <span className="kv-v">{coreVersion ?? "—"}</span>
            </div>
            <div>
              <span className="kv-k">{t("dashboard.routing")}</span>
              <span className="kv-v">{modeLabel}</span>
            </div>
          </div>
        </article>

        <article
          className="instrument accent-blue instrument-click"
          role="button"
          tabIndex={0}
          onClick={() => onGoTraffic?.()}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") onGoTraffic?.();
          }}
        >
          <header className="instrument-head">
            <span className="instrument-label">{t("dashboard.cardTraffic")}</span>
            <span className="instrument-tag">NET</span>
          </header>
          <div className="instrument-traffic">
            <div>
              <span className="tr-dir">↓</span>{" "}
              {fmtSpeed(proxy?.download_speed ?? 0)}
            </div>
            <div>
              <span className="tr-dir">↑</span>{" "}
              {fmtSpeed(proxy?.upload_speed ?? 0)}
            </div>
          </div>
          <div className="instrument-kv mono">
            <div>
              <span className="kv-k">Σ ↓</span>
              <span className="kv-v">
                {fmtBytes(proxy?.download_total ?? 0)}
              </span>
            </div>
            <div>
              <span className="kv-k">Σ ↑</span>
              <span className="kv-v">{fmtBytes(proxy?.upload_total ?? 0)}</span>
            </div>
          </div>
        </article>

        <article
          className="instrument accent-cyan instrument-click"
          role="button"
          tabIndex={0}
          onClick={() => onGoNodes?.()}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") onGoNodes?.();
          }}
        >
          <header className="instrument-head">
            <span className="instrument-label">
              {t("dashboard.cardQuality")}
            </span>
            <span className="instrument-tag">
              {latencyStats.n > 0 ? `${latencyStats.n}` : "—"}
            </span>
          </header>
          <div className="instrument-value sm mono">
            {fmtLatency(currentNode?.latency_ms)}
          </div>
          <div className="instrument-kv mono">
            <div>
              <span className="kv-k">{t("dashboard.latencyNow")}</span>
              <span className="kv-v">
                {fmtLatency(currentNode?.latency_ms)}
              </span>
            </div>
            <div>
              <span className="kv-k">{t("dashboard.latencyAvg")}</span>
              <span className="kv-v">{fmtLatency(latencyStats.avg)}</span>
            </div>
            <div>
              <span className="kv-k">{t("dashboard.latencyBest")}</span>
              <span className="kv-v">{fmtLatency(latencyStats.best)}</span>
            </div>
          </div>
        </article>

        <article
          className="instrument accent-yellow instrument-click"
          role="button"
          tabIndex={0}
          onClick={() => onGoTraffic?.()}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") onGoTraffic?.();
          }}
        >
          <header className="instrument-head">
            <span className="instrument-label">
              {t("dashboard.cardConns")}
            </span>
            <span className="instrument-tag">LIVE</span>
          </header>
          <div className="instrument-value">
            {proxy?.connections ?? 0}
          </div>
          <div className="instrument-sub mono">
            {t("dashboard.activeConns")}
          </div>
        </article>

        <article
          className="instrument accent-green instrument-click"
          role="button"
          tabIndex={0}
          onClick={() => onGoProfiles?.()}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") onGoProfiles?.();
          }}
        >
          <header className="instrument-head">
            <span className="instrument-label">
              {t("dashboard.cardSub")}
            </span>
            <span className="instrument-tag">
              {subCount > 0 ? "ACTIVE" : "—"}
            </span>
          </header>
          <div className="instrument-value sm">
            {activeSub?.name ?? t("dashboard.noSub")}
          </div>
          <div className="instrument-kv mono">
            <div>
              <span className="kv-k">{t("dashboard.profiles")}</span>
              <span className="kv-v">
                {subCount} · {nodeCount} {t("dashboard.nodes").toLowerCase()}
              </span>
            </div>
            <div>
              <span className="kv-k">{t("dashboard.updated")}</span>
              <span className="kv-v">
                {activeSub
                  ? relativeAgo(activeSub.last_update, t)
                  : "—"}
              </span>
            </div>
            <div>
              <span className="kv-k">{t("dashboard.quota")}</span>
              <span className="kv-v">{subQuotaLabel}</span>
            </div>
          </div>
        </article>

        <article
          className="instrument accent-cyan instrument-click"
          role="button"
          tabIndex={0}
          onClick={() => onGoSettings?.()}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") onGoSettings?.();
          }}
        >
          <header className="instrument-head">
            <span className="instrument-label">
              {t("dashboard.cardSystem")}
            </span>
            <span className="instrument-tag">I/O</span>
          </header>
          <div className="instrument-value sm mono">
            mixed :{mixedPort}
          </div>
          <div className="instrument-kv mono">
            <div>
              <span className="kv-k">API</span>
              <span className="kv-v">:{settingsPorts.api}</span>
            </div>
            <div>
              <span className="kv-k">SYS</span>
              <span className="kv-v">
                {proxy?.system_proxy ? "ON" : "OFF"}
              </span>
            </div>
            <div>
              <span className="kv-k">TUN</span>
              <span className="kv-v">
                {proxy?.tun_enabled ? "ON" : "OFF"}
              </span>
            </div>
          </div>
        </article>
      </section>

      {showPreview && result && (
        <div className="card glass preview-card">
          <div className="preview-head">
            <strong>{t("common.preview")}</strong>
            <button
              type="button"
              className="secondary small"
              onClick={() => setShowPreview(false)}
            >
              {t("common.close")}
            </button>
          </div>
          <pre className="preview-json">{result.preview}</pre>
        </div>
      )}
    </div>
  );
}
