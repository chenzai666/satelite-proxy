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
  startProxy,
  smartSwitchNow,
  stopProxy,
  updateSettings,
} from "../api";
import {
  useCaptureModeSwitch,
} from "../hooks/useCaptureModeSwitch";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import { useI18n } from "../i18n";
import { GlassSeg } from "../components/GlassSeg";
import { HeroVisual } from "../components/HeroVisual";
import { SimpleTrafficSpark } from "../ui/simple/SimpleTrafficSpark";
import type {
  AutoSelectMode,
  GenerateConfigResult,
  OutboundMode,
  ProxyNode,
  ProxyStatus,
  SubscriptionTraffic,
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
  if (!Number.isFinite(n) || n < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  if (i === 0) return `${Math.round(v)} B`;
  const text = v >= 100 ? String(Math.round(v)) : v.toFixed(1);
  return `${text} ${units[i]}`;
}

function quotaParts(tr: SubscriptionTraffic | null | undefined) {
  if (!tr) return null;
  const total = tr.total != null && tr.total > 0 ? tr.total : null;
  const remaining =
    tr.quota_remaining != null && tr.quota_remaining >= 0
      ? tr.quota_remaining
      : null;
  if (total == null && remaining == null) return null;
  const usedParts = (tr.upload ?? 0) + (tr.download ?? 0);
  const used =
    total != null && usedParts === 0 && remaining != null
      ? Math.max(0, total - remaining)
      : usedParts;
  return { used, total, remaining };
}

function fmtLatency(ms?: number | null) {
  if (ms == null || ms < 0) return "—";
  return `${ms} ms`;
}

function latencyClass(ms?: number | null) {
  if (ms == null || ms < 0) return "lat-none";
  if (ms < 200) return "lat-good";
  if (ms < 300) return "lat-ok";
  return "lat-slow";
}

function latencyLevel(ms?: number | null) {
  if (ms == null || ms < 0) return null;
  if (ms < 200) return "good";
  if (ms < 300) return "ok";
  return "slow";
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
  const [mixMode, setMixMode] = useState(false);
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
  /** Bootstrap probe after enabling smart switch (does not lock other controls). */
  const [smartProbing, setSmartProbing] = useState(false);
  const smartGenRef = useRef(0);
  const [modeBusy, setModeBusy] = useState(false);
  const [envCopied, setEnvCopied] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);
  const [spark, setSpark] = useState<
    { up: number; down: number; conns: number }[]
  >([]);

  const pushSpark = useCallback((s: ProxyStatus | null) => {
    setSpark((prev) => {
      const next = [
        ...prev,
        {
          up: s?.upload_speed ?? 0,
          down: s?.download_speed ?? 0,
          conns: s?.connections ?? 0,
        },
      ];
      return next.length > 60 ? next.slice(next.length - 60) : next;
    });
  }, []);

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
      setMixMode(!!settings.mix_mode);
      setCurrentNodeId(settings.current_node_id ?? null);
      setProxy(status);
      pushSpark(status);
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
  }, [pushSpark, t]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const onCaptureError = useCallback((msg: string) => {
    setError(msg);
  }, []);

  // Hook only invokes this when the drain batch touched TUN (core restart).
  const onCaptureApplied = useCallback(() => {
    void reload();
  }, [reload]);

  const { captureMode, captureBusy, requestCaptureMode } = useCaptureModeSwitch(
    proxy,
    setProxy,
    onCaptureError,
    onCaptureApplied,
  );

  useVisibleInterval(() => {
    // Do not clobber optimistic capture UI while a switch is in flight.
    if (captureBusy) return;
    return getProxyStatus()
      .then((s) => {
        setProxy(s);
        pushSpark(s);
      })
      .catch(() => undefined);
  }, 1000);

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

  const enabledSubs = useMemo(() => subs.filter((s) => s.enabled), [subs]);

  const activeSub = enabledSubs[0] ?? subs[0] ?? null;

  const visibleSubs = useMemo(
    () => (enabledSubs.length > 0 ? enabledSubs : activeSub ? [activeSub] : []),
    [activeSub, enabledSubs],
  );

  const activeSubNames = useMemo(
    () => visibleSubs.map((sub) => sub.name).join(" · "),
    [visibleSubs],
  );

  const subQuota = useMemo(() => {
    const empty = {
      used: 0,
      total: null as number | null,
      remaining: null as number | null,
      ratio: null as number | null,
      label: "—",
    };
    const parts = visibleSubs
      .map((sub) => quotaParts(sub.traffic))
      .filter((p): p is NonNullable<typeof p> => p != null);
    if (parts.length === 0) return empty;

    const withTotal = parts.filter((p) => p.total != null);
    if (withTotal.length > 0) {
      const used = withTotal.reduce((sum, p) => sum + p.used, 0);
      const total = withTotal.reduce((sum, p) => sum + (p.total ?? 0), 0);
      const ratio = total > 0 ? Math.min(1, used / total) : 0;
      return {
        used,
        total,
        remaining: Math.max(0, total - used),
        ratio,
        label: `${fmtBytes(used)} / ${fmtBytes(total)}`,
      };
    }

    const withRemaining = parts.filter((p) => p.remaining != null);
    if (withRemaining.length > 0) {
      const remaining = withRemaining.reduce(
        (sum, p) => sum + (p.remaining ?? 0),
        0,
      );
      return {
        used: 0,
        total: null,
        remaining,
        ratio: null,
        label: t("common.remaining", { n: fmtBytes(remaining) }),
      };
    }
    return empty;
  }, [t, visibleSubs]);

  const quotaPct =
    subQuota.ratio != null ? Math.round(subQuota.ratio * 100) : null;
  const quotaLevel =
    subQuota.ratio == null
      ? ""
      : subQuota.ratio >= 0.9
        ? "critical"
        : subQuota.ratio >= 0.7
          ? "warn"
          : "ok";

  const modeLabel =
    outboundMode === "rule"
      ? t("dashboard.modeRule")
      : outboundMode === "global"
        ? t("dashboard.modeGlobal")
        : t("dashboard.modeDirect");

  const currentLatency = currentNode?.latency_ms;
  const qualityLevel = latencyLevel(currentLatency);
  const qualityTag =
    qualityLevel === "good"
      ? "GOOD"
      : qualityLevel === "ok"
        ? "OK"
        : qualityLevel === "slow"
          ? "SLOW"
          : "—";
  const qualityTagClass =
    qualityLevel === "good"
      ? " ok"
      : qualityLevel === "ok"
        ? " warn"
        : qualityLevel === "slow"
          ? " err"
          : "";

  return (
    <div className="page dashboard-page">
      {toast && <div className="toast">{toast}</div>}
      {error && <div className="banner error">{error}</div>}
      {proxy?.error && !running && (
        <div className="banner error">core: {proxy.error}</div>
      )}

      {/* —— Hero: orbit + status + embedded controls (no floating QC card) —— */}
      <section className={`dash-hero is-${orbitState}`}>
        <HeroVisual
          state={orbitState}
          spinning={running || switching}
          switching={switching}
        />

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
                    {busy && running ? (
                      <>
                        <span
                          className="lat-spinner ui-mode-restart-spinner"
                          aria-hidden
                        />{" "}
                        {t("dashboard.restart")}
                      </>
                    ) : (
                      t("dashboard.restart")
                    )}
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
            <GlassSeg
              value={outboundMode}
              ready={statusReady}
              ariaLabel={t("dashboard.routing")}
              disabled={controlsBusy || !statusReady}
              onChange={(v) => void onSetMode(v as OutboundMode)}
              options={[
                { value: "rule", label: t("dashboard.modeRule") },
                { value: "global", label: t("dashboard.modeGlobal") },
                { value: "direct", label: t("dashboard.modeDirect") },
              ]}
            />
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
            <GlassSeg
              value={autoSelectMode}
              ready={statusReady}
              ariaLabel={t("dashboard.autoSelect")}
              disabled={modeBusy || !statusReady}
              disabledValues={
                new Set(
                  [
                    smartProbing ? "smart" : null,
                    nodeCount === 0 &&
                    autoSelectMode === "off" &&
                    !smartProbing
                      ? "kernel"
                      : null,
                    nodeCount === 0 &&
                    autoSelectMode === "off" &&
                    !smartProbing
                      ? "smart"
                      : null,
                  ].filter((v): v is string => v != null),
                )
              }
              titles={{
                kernel: t("dashboard.autoSelectKernelHint"),
                smart: t("dashboard.smartSwitchDesc"),
                off: t("dashboard.autoSelectDesc"),
              }}
              onChange={(v) => void onSetAutoSelect(v as AutoSelectMode)}
              options={[
                { value: "off", label: t("dashboard.autoSelectOff") },
                { value: "kernel", label: t("dashboard.autoSelectKernel") },
                { value: "smart", label: t("dashboard.autoSelectSmart") },
              ]}
            />
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
            <GlassSeg
              value={captureMode}
              ready={statusReady}
              ariaLabel={t("dashboard.capture")}
              disabled={!statusReady}
              disabledValues={
                new Set(
                  [
                    nodeCount === 0 && captureMode !== "tun" ? "tun" : null,
                  ].filter((v): v is string => v != null),
                )
              }
              titles={{
                tun: t("dashboard.captureTunHint"),
                system: t("dashboard.captureSystemHint"),
                off: t("dashboard.captureDesc"),
              }}
              onChange={(v) => {
                setError(null);
                requestCaptureMode(v as "off" | "system" | "tun");
              }}
              options={[
                { value: "off", label: t("dashboard.captureOff") },
                { value: "system", label: t("dashboard.captureSystem") },
                { value: "tun", label: t("dashboard.captureTun") },
              ]}
            />
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

      {/* —— 6 cards: core / spark / traffic+conns · quality / sub / system —— */}
      <section className="instrument-grid instrument-grid-6" aria-label="Telemetry">
        <article className="instrument accent-green">
          <header className="instrument-head">
            <span className="instrument-label">{t("dashboard.cardCore")}</span>
            <span
              className={`instrument-tag ${running ? "ok" : isError ? "err" : ""}`}
            >
              {running
                ? "ONLINE"
                : switching
                  ? "…"
                  : isError
                    ? "ERR"
                    : "IDLE"}
            </span>
          </header>
          <div
            className={`instrument-value readout${
              running
                ? ""
                : switching
                  ? " state-busy"
                  : isError
                    ? " state-error"
                    : " state-off"
            }`}
          >
            {running
              ? t("dashboard.coreRunning")
              : switching
                ? stateLabel === "stopping"
                  ? t("dashboard.coreStopping")
                  : t("dashboard.coreStarting")
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

        <SimpleTrafficSpark
          samples={spark}
          up={proxy?.upload_speed ?? 0}
          down={proxy?.download_speed ?? 0}
          conns={proxy?.connections ?? 0}
          running={running}
          label={t("simple.spark")}
          idleLabel={t("simple.sparkIdle")}
          idleConnsLabel={t("simple.sparkIdleConns")}
          connsLabel={t("simple.sparkConns", { n: proxy?.connections ?? 0 })}
          onOpen={onGoTraffic}
        />

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
            <span className="instrument-label">
              {t("dashboard.cardTraffic")} · {t("dashboard.cardConns")}
            </span>
            <span className="instrument-tag">
              {proxy?.connections ?? 0}
            </span>
          </header>
          <div className="instrument-traffic-cols">
            <div className="instrument-traffic-col">
              <div className="instrument-traffic-col-label">
                {t("dashboard.trafficLive")}
              </div>
              <div className="instrument-traffic">
                <div>
                  <span className="tr-dir down">↓</span>{" "}
                  {fmtSpeed(proxy?.download_speed ?? 0)}
                </div>
                <div>
                  <span className="tr-dir up">↑</span>{" "}
                  {fmtSpeed(proxy?.upload_speed ?? 0)}
                </div>
              </div>
            </div>
            <div className="instrument-traffic-col">
              <div className="instrument-traffic-col-label">
                {t("dashboard.trafficTotal")}
              </div>
              <div className="instrument-traffic">
                <div>
                  <span className="tr-sigma down">Σ</span>
                  <span className="tr-dir down">↓</span>{" "}
                  {fmtBytes(proxy?.download_total ?? 0)}
                </div>
                <div>
                  <span className="tr-sigma up">Σ</span>
                  <span className="tr-dir up">↑</span>{" "}
                  {fmtBytes(proxy?.upload_total ?? 0)}
                </div>
              </div>
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
            <span className={`instrument-tag${qualityTagClass}`}>
              {qualityTag}
            </span>
          </header>
          <div
            className={`instrument-value readout mono ${latencyClass(currentLatency)}`}
          >
            {fmtLatency(currentLatency)}
          </div>
          <div className="instrument-kv mono">
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
            <span className={`instrument-tag${mixMode ? " ok" : ""}`}>
              {subCount === 0
                ? "—"
                : mixMode
                  ? `${t("config.mix")} ${enabledSubs.length}/${subCount}`
                  : "ACTIVE"}
            </span>
          </header>
          <div
            className={`instrument-value readout instrument-subscription-names${
              visibleSubs.length > 1 || (activeSubNames?.length ?? 0) > 12
                ? " wrap"
                : ""
            }`}
            title={activeSubNames || undefined}
          >
            <span>{activeSubNames || t("dashboard.noSub")}</span>
          </div>
          <div className="instrument-kv mono">
            <div className="instrument-quota-row">
              <span className="kv-k">{t("dashboard.quota")}</span>
              {quotaPct != null ? (
                <span
                  className={`instrument-quota-bar ${quotaLevel}`}
                  title={`${quotaPct}%`}
                  aria-label={`${quotaPct}%`}
                >
                  <span
                    className="instrument-quota-fill"
                    style={{ width: `${quotaPct}%` }}
                  />
                </span>
              ) : null}
              <span className="kv-v">{subQuota.label}</span>
            </div>
            <div>
              <span className="kv-k">{t("dashboard.profiles")}</span>
              <span className="kv-v">
                {subCount} · {nodeCount} {t("dashboard.nodes").toLowerCase()}
              </span>
            </div>
          </div>
        </article>

        <article
          className="instrument accent-cyan instrument-click"
          role="button"
          tabIndex={0}
          title={t("dashboard.copyEnvHint")}
          onClick={() => void onCopyEnv()}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              void onCopyEnv();
            }
          }}
        >
          <header className="instrument-head">
            <span className="instrument-label">
              {t("dashboard.cardSystem")}
            </span>
            <span
              className={`instrument-tag${envCopied ? " ok" : ""}`}
              aria-hidden
            >
              {envCopied ? "✓" : "⧉"}
            </span>
          </header>
          <div className="instrument-value readout mono">:{mixedPort}</div>
          <div className="instrument-kv mono">
            <div>
              <span className="kv-k">PROXY</span>
              <span className="kv-v">http://127.0.0.1:{mixedPort}</span>
            </div>
            <div>
              <span className="kv-k">ENV</span>
              <span className="kv-v">
                {envCopied
                  ? t("dashboard.envCopied")
                  : t("dashboard.copyEnvHint")}
              </span>
            </div>
          </div>
        </article>
      </section>

      {showPreview && result && (
        <div
          className="modal-backdrop"
        >
          <div
            className="modal preview-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="preview-modal-title"
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
