import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import {
  getCoreInfo,
  getProxyStatus,
  getSettings,
  listAllNodes,
  listSubscriptions,
  previewSingboxConfig,
  restartProxy,
  setCaptureMode,
  setOutboundMode,
  startProxy,
  smartSwitchNow,
  stopProxy,
  updateSettings,
} from "../api";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import { useI18n } from "../i18n";
import type {
  AutoSelectMode,
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
  /** settings.current_node_id — available before full node list. */
  const [currentNodeId, setCurrentNodeId] = useState<string | null>(null);
  const [settingsPorts, setSettingsPorts] = useState({ mixed: 2080, api: 19090 });
  const [coreLabel, setCoreLabel] = useState("—");
  const [coreVersion, setCoreVersion] = useState<string | null>(null);
  const [proxy, setProxy] = useState<ProxyStatus | null>(null);
  /** false until status wave lands; details (nodes/subs) may still be loading. */
  const [statusReady, setStatusReady] = useState(false);
  const [detailsReady, setDetailsReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<GenerateConfigResult | null>(null);
  const [showPreview, setShowPreview] = useState(false);
  const [captureBusy, setCaptureBusy] = useState(false);
  /** Optimistic selection while backend applies (null = use proxy status). */
  const [captureUi, setCaptureUi] = useState<"off" | "system" | "tun" | null>(
    null,
  );
  const captureGenRef = useRef(0);
  /** Bootstrap probe after enabling smart switch (does not lock other controls). */
  const [smartProbing, setSmartProbing] = useState(false);
  const smartGenRef = useRef(0);
  const [modeBusy, setModeBusy] = useState(false);
  const [envCopied, setEnvCopied] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);

  /** Full reload (actions after start/stop/etc). */
  const reload = useCallback(async () => {
    setError(null);
    try {
      // Kick both waves at once; commit status as soon as wave 1 resolves.
      const statusP = Promise.all([
        getSettings(),
        getProxyStatus().catch(() => null),
      ]);
      const detailP = Promise.all([
        listSubscriptions(),
        listAllNodes(),
        getCoreInfo().catch(() => null),
      ]);

      const [settings, status] = await statusP;
      setSettingsPorts({ mixed: settings.mixed_port, api: settings.api_port });
      setCurrentNodeId(settings.current_node_id ?? null);
      setProxy(status);
      setStatusReady(true);

      const [subList, nodeList, core] = await detailP;
      setSubs(subList);
      setNodes(nodeList);
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
      setDetailsReady(true);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      setStatusReady(true);
      setDetailsReady(true);
    }
  }, [t]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useVisibleInterval(() => {
    // Do not clobber optimistic capture UI while a switch is in flight.
    if (captureBusy) return;
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

  type CaptureMode = "off" | "system" | "tun";

  function resolveCaptureMode(p: ProxyStatus | null): CaptureMode {
    if (p?.tun_enabled) return "tun";
    if (p?.system_proxy) return "system";
    return "off";
  }

  const captureMode = captureUi ?? resolveCaptureMode(proxy);

  /**
   * Optimistic UI: paint target mode immediately, apply in background.
   * Failure → restore previous selection + error banner.
   */
  function onSetCaptureMode(mode: CaptureMode) {
    if (mode === captureMode || captureBusy) return;
    const prevProxy = proxy;
    const prevMode = resolveCaptureMode(proxy);
    const gen = ++captureGenRef.current;

    // Force commit before IPC so the indicator slides this frame.
    flushSync(() => {
      setCaptureUi(mode);
      setCaptureBusy(true);
      setError(null);
      if (prevProxy) {
        setProxy({
          ...prevProxy,
          system_proxy: mode === "system",
          tun_enabled: mode === "tun",
        });
      }
    });

    void (async () => {
      try {
        const s = await setCaptureMode(mode);
        if (gen !== captureGenRef.current) return;
        setProxy(s);
        setCaptureUi(null);
        if (mode === "tun" || prevMode === "tun") {
          void reload();
        }
      } catch (e) {
        if (gen !== captureGenRef.current) return;
        setError(typeof e === "string" ? e : String(e));
        setCaptureUi(null);
        if (prevProxy) {
          setProxy(prevProxy);
        } else {
          const s = await getProxyStatus().catch(() => null);
          if (s) setProxy(s);
        }
      } finally {
        if (gen === captureGenRef.current) {
          setCaptureBusy(false);
        }
      }
    })();
  }

  function resolveAutoSelect(p: ProxyStatus | null): AutoSelectMode {
    const raw = (p?.auto_select ?? (p?.smart_switch ? "smart" : "off")) as string;
    if (raw === "smart" || raw === "kernel") return raw;
    return "off";
  }

  async function onSetAutoSelect(mode: AutoSelectMode) {
    if (mode === autoSelectMode) return;
    setError(null);
    const prev = autoSelectMode;

    // Leaving smart: cancel any in-flight bootstrap probe.
    if (mode !== "smart") {
      smartGenRef.current += 1;
      setSmartProbing(false);
    }

    setProxy((p) =>
      p
        ? {
            ...p,
            auto_select: mode,
            smart_switch: mode === "smart",
          }
        : p,
    );

    const gen = ++smartGenRef.current;
    if (mode === "smart") setSmartProbing(true);

    try {
      await updateSettings({ autoSelect: mode });
      if (gen !== smartGenRef.current) return;

      if (mode === "smart") {
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
      }

      if (gen !== smartGenRef.current) return;
      await reload();
      const s = await getProxyStatus().catch(() => null);
      if (s) setProxy(s);
    } catch (e) {
      if (gen === smartGenRef.current) {
        setError(typeof e === "string" ? e : String(e));
        setProxy((p) =>
          p
            ? {
                ...p,
                auto_select: prev,
                smart_switch: prev === "smart",
              }
            : p,
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
  // captureBusy must NOT freeze other controls (optimistic capture runs long).
  const controlsBusy = busy || modeBusy;
  const autoSelectMode = resolveAutoSelect(proxy);
  const nodeCount = nodes.length;
  const subCount = subs.length;
  // Allow start once we know a node id, even if full list is still loading.
  const canStart =
    nodeCount > 0 || (!!currentNodeId && statusReady);
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

  const heroTitle = !detailsReady && running
    ? null // skeleton
    : running
      ? currentNode?.name ?? t("dashboard.disconnected")
      : isError
        ? t("dashboard.errorTitle")
        : t("dashboard.disconnected");

  const heroSub = !detailsReady && running
    ? null
    : running
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
    const proxyUrl = `http://127.0.0.1:${mixedPort}`;
    const isWindows = /Windows/i.test(navigator.userAgent);
    const text = isWindows
      ? `$env:ALL_PROXY = "${proxyUrl}"`
      : `export all_proxy=${proxyUrl}`;
    try {
      await navigator.clipboard.writeText(text);
      setEnvCopied(true);
      setMoreOpen(false);
      setToast(t("dashboard.envCopied"));
      window.setTimeout(() => setEnvCopied(false), 1500);
      window.setTimeout(() => setToast(null), 1500);
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
      {toast && <div className="toast">{toast}</div>}
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

          <h1 className="dash-hero-title">
            {heroTitle == null ? (
              <span className="skel skel-inline skel-w-40" aria-hidden />
            ) : (
              heroTitle
            )}
          </h1>
          <p className="dash-hero-desc">
            {heroSub == null ? (
              <span className="skel skel-inline skel-w-30" aria-hidden />
            ) : (
              heroSub
            )}
          </p>

          <div className="dash-hero-actions">
            {!running ? (
              <button
                type="button"
                className="btn-pill"
                disabled={busy || !canStart || switching || !statusReady}
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
              disabled={!canStart}
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
                    disabled={busy || !canStart}
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
          <div className="dash-inline-row dash-auto-select">
            <span
              className={`dash-inline-label${smartProbing ? " dash-smart-probing" : ""}`}
              title={
                smartProbing
                  ? t("dashboard.smartSwitchProbing")
                  : t("dashboard.autoSelectDesc")
              }
            >
              {smartProbing ? (
                <>
                  <span className="lat-spinner dash-smart-spinner" aria-hidden />
                  <span>{t("dashboard.smartSwitchProbing")}</span>
                </>
              ) : (
                t("dashboard.autoSelect")
              )}
            </span>
            <div
              className="segmented compact mode-seg dash-inline-seg dash-auto-seg"
              role="group"
              aria-label={t("dashboard.autoSelect")}
              aria-busy={smartProbing}
            >
              <span
                className="seg-indicator"
                aria-hidden="true"
                style={{
                  transform: `translateX(${
                    autoSelectMode === "off"
                      ? 0
                      : autoSelectMode === "kernel"
                        ? 100
                        : 200
                  }%)`,
                }}
              />
              {(
                [
                  ["off", t("dashboard.autoSelectOff")],
                  ["kernel", t("dashboard.autoSelectKernel")],
                  ["smart", t("dashboard.autoSelectSmart")],
                ] as const
              ).map(([key, label]) => (
                <button
                  key={key}
                  type="button"
                  className={`seg ${autoSelectMode === key ? "active" : ""}`}
                  disabled={
                    modeBusy ||
                    // While probing: allow Off / Kernel to cancel; only block re-tapping Smart.
                    (smartProbing && key === "smart") ||
                    (nodeCount === 0 &&
                      key !== "off" &&
                      autoSelectMode === "off" &&
                      !smartProbing)
                  }
                  title={
                    key === "kernel"
                      ? t("dashboard.autoSelectKernelHint")
                      : key === "smart"
                        ? t("dashboard.smartSwitchDesc")
                        : t("dashboard.autoSelectDesc")
                  }
                  onClick={() => void onSetAutoSelect(key)}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>
          <div className="dash-inline-row dash-auto-select dash-capture">
            <span
              className={`dash-inline-label${captureBusy ? " dash-smart-probing" : ""}`}
              title={
                captureBusy
                  ? t("dashboard.captureSwitching")
                  : t("dashboard.captureDesc")
              }
            >
              {captureBusy ? (
                <>
                  <span className="lat-spinner dash-smart-spinner" aria-hidden />
                  <span>{t("dashboard.captureSwitching")}</span>
                </>
              ) : (
                t("dashboard.capture")
              )}
            </span>
            <div
              className="segmented compact mode-seg dash-inline-seg dash-auto-seg"
              role="group"
              aria-label={t("dashboard.capture")}
              aria-busy={captureBusy}
            >
              <span
                className="seg-indicator"
                aria-hidden="true"
                style={{
                  transform: `translateX(${
                    captureMode === "off"
                      ? 0
                      : captureMode === "system"
                        ? 100
                        : 200
                  }%)`,
                }}
              />
              {(
                [
                  ["off", t("dashboard.captureOff")],
                  ["system", t("dashboard.captureSystem")],
                  ["tun", t("dashboard.captureTun")],
                ] as const
              ).map(([key, label]) => (
                <button
                  key={key}
                  type="button"
                  className={`seg ${captureMode === key ? "active" : ""}`}
                  disabled={
                    (captureBusy && key !== captureMode) ||
                    (key === "tun" && nodeCount === 0 && captureMode !== "tun")
                  }
                  title={
                    key === "tun"
                      ? t("dashboard.captureTunHint")
                      : key === "system"
                        ? t("dashboard.captureSystemHint")
                        : t("dashboard.captureDesc")
                  }
                  onClick={() => onSetCaptureMode(key)}
                >
                  {label}
                </button>
              ))}
            </div>
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
              <span className="kv-k">{t("dashboard.capture")}</span>
              <span className="kv-v">
                {proxy?.tun_enabled
                  ? t("dashboard.captureTun")
                  : proxy?.system_proxy
                    ? t("dashboard.captureSystem")
                    : t("dashboard.captureOff")}
              </span>
            </div>
          </div>
        </article>
      </section>

      {showPreview && result && (
        <div
          className="modal-backdrop"
          onClick={() => setShowPreview(false)}
        >
          <div
            className="modal preview-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="preview-modal-title"
            onClick={(e) => e.stopPropagation()}
          >
            <header className="modal-header">
              <h2 id="preview-modal-title">{t("common.preview")}</h2>
              <button
                type="button"
                className="icon-btn"
                onClick={() => setShowPreview(false)}
                aria-label={t("common.close")}
              >
                ×
              </button>
            </header>
            <div className="modal-body">
              <pre className="preview-json">{result.preview}</pre>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
