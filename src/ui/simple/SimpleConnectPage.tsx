import { useCallback, useEffect, useRef, useState } from "react";
import {
  getProxyStatus,
  getSettings,
  listAllNodes,
  setOutboundMode,
  smartSwitchNow,
  startProxy,
  stopProxy,
  testNodesLatency,
  updateSettings,
} from "../../api";
import { GlassSeg } from "../../components/GlassSeg";
import { HeroVisual } from "../../components/HeroVisual";
import { useCaptureModeSwitch } from "../../hooks/useCaptureModeSwitch";
import { useVisibleInterval } from "../../hooks/useVisibleInterval";
import { useI18n } from "../../i18n";
import { SimpleTrafficSpark } from "./SimpleTrafficSpark";
import type {
  AutoSelectMode,
  OutboundMode,
  ProxyNode,
  ProxyStatus,
} from "../../types";

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

function latencyClass(ms?: number | null) {
  if (ms == null || ms < 0) return "lat-none";
  if (ms < 200) return "lat-good";
  if (ms < 300) return "lat-ok";
  return "lat-slow";
}

interface Props {
  onGoServers?: () => void;
  onGoTraffic?: () => void;
}

export function SimpleConnectPage({ onGoServers, onGoTraffic }: Props) {
  const { t } = useI18n();
  const [proxy, setProxy] = useState<ProxyStatus | null>(null);
  const [node, setNode] = useState<ProxyNode | null>(null);
  const [nodeCount, setNodeCount] = useState(0);
  const [nodeReady, setNodeReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [smartProbing, setSmartProbing] = useState(false);
  const smartGenRef = useRef(0);
  const [nowSec, setNowSec] = useState(() => Math.floor(Date.now() / 1000));
  const [spark, setSpark] = useState<
    { up: number; down: number; conns: number }[]
  >([]);

  const reloadStatus = useCallback(async () => {
    try {
      const status = await getProxyStatus().catch(() => null);
      setProxy(status);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, []);

  const reloadNode = useCallback(async () => {
    try {
      const [settings, nodes] = await Promise.all([
        getSettings().catch(() => null),
        listAllNodes().catch(() => [] as ProxyNode[]),
      ]);
      const id = settings?.current_node_id;
      setNode(id ? (nodes.find((n) => n.id === id) ?? null) : nodes[0] ?? null);
      setNodeCount(nodes.length);
      setNodeReady(true);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      setNodeReady(true);
    }
  }, []);

  const reload = useCallback(async () => {
    const statusP = reloadStatus();
    const nodeP = reloadNode();
    await statusP;
    await nodeP;
  }, [reloadStatus, reloadNode]);

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
      /* keep prior / cleared state */
    } finally {
      setTesting(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  useVisibleInterval(() => {
    setNowSec(Math.floor(Date.now() / 1000));
    return reloadStatus();
  }, 1000);

  const onCaptureError = useCallback((msg: string) => {
    setError(msg);
  }, []);

  const { captureMode, captureBusy, requestCaptureMode } = useCaptureModeSwitch(
    proxy,
    setProxy,
    onCaptureError,
  );

  const running = proxy?.running ?? false;
  const connecting =
    busy ||
    proxy?.core_state === "starting" ||
    proxy?.core_state === "stopping";
  const isError = proxy?.core_state === "error" || (!!proxy?.error && !running);
  const orbitState = connecting
    ? "switching"
    : running
      ? "live"
      : isError
        ? "error"
        : "stopped";
  const stateUpper = running
    ? "RUNNING"
    : connecting
      ? proxy?.core_state === "stopping" || (busy && running)
        ? "STOPPING"
        : "STARTING"
      : isError
        ? "ERROR"
        : "STOPPED";
  const dotClass = running ? "on" : connecting ? "busy" : "off";

  function resolveAutoSelect(): AutoSelectMode {
    const raw =
      proxy?.auto_select ?? (proxy?.smart_switch ? "smart" : "off");
    if (raw === "smart" || raw === "kernel") return raw;
    return "off";
  }

  const autoSelectMode = resolveAutoSelect();
  const outboundMode = (proxy?.outbound_mode ?? "rule") as OutboundMode;
  const customRuntime = proxy?.runtime_source === "singbox";

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
        const id = node?.id;
        if (id) void probeCurrent(id);
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onSetAutoSelect(mode: AutoSelectMode) {
    const prev = autoSelectMode;
    if (mode === prev) return;
    setError(null);
    if (mode !== "smart") {
      smartGenRef.current += 1;
      setSmartProbing(false);
    }
    setProxy((p) =>
      p ? { ...p, auto_select: mode, smart_switch: mode === "smart" } : p,
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
          } else if (
            r.message === "all probes failed" ||
            r.message === "clash api unavailable"
          ) {
            setError(t("dashboard.smartSwitchProbeFail"));
          } else if (r.message === "no nodes") {
            setError(t("dashboard.smartSwitchNoNodes"));
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
    } catch (e) {
      if (gen === smartGenRef.current) {
        setError(typeof e === "string" ? e : String(e));
        setProxy((p) =>
          p
            ? { ...p, auto_select: prev, smart_switch: prev === "smart" }
            : p,
        );
      }
    } finally {
      if (gen === smartGenRef.current) setSmartProbing(false);
    }
  }

  async function onSetMode(mode: OutboundMode) {
    setBusy(true);
    setError(null);
    try {
      setProxy(await setOutboundMode(mode));
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  const up = proxy?.upload_speed ?? 0;
  const down = proxy?.download_speed ?? 0;
  const conns = proxy?.connections ?? 0;

  useEffect(() => {
    setSpark((prev) => {
      const next = [...prev, { up, down, conns }];
      return next.length > 60 ? next.slice(next.length - 60) : next;
    });
  }, [nowSec, up, down, conns]);
  const startedAt = proxy?.core_started_at ?? null;
  const uptimeLabel =
    running && startedAt != null && startedAt > 0
      ? fmtUptime(nowSec - startedAt)
      : "—";

  const heroTitle = !nodeReady && running
    ? null
    : customRuntime
      ? t("dashboard.customMode", {
          name: proxy?.runtime_profile_name || t("config.singbox"),
        })
      : running
        ? node?.name ?? t("dashboard.disconnected")
        : isError
          ? t("dashboard.errorTitle")
          : t("dashboard.disconnected");

  const heroSub = !nodeReady && running
    ? null
    : customRuntime
      ? t("config.singboxReadonly")
      : running
        ? [node?.protocol?.toUpperCase(), testing ? "…" : fmtLatency(node?.latency_ms)]
            .filter(Boolean)
            .join(" · ")
        : t("dashboard.desc");

  return (
    <div className="simple-page simple-connect">
      {error && <div className="banner error">{error}</div>}

      <section className={`dash-hero simple-dash-hero is-${orbitState}`}>
        <button
          type="button"
          className="simple-orbit-btn"
          disabled={busy || connecting}
          onClick={() => void onToggle()}
          aria-label={running ? t("dashboard.stop") : t("dashboard.start")}
          aria-pressed={running}
        >
          <HeroVisual
            state={orbitState}
            spinning={running || connecting}
            switching={connecting}
            variant="simple"
          />
        </button>
        <div className="dash-hero-copy simple-dash-copy">
          <div className="dash-kicker mono">
            <span className={`status-dot ${dotClass}`} />
            {stateUpper}
            {running && (
              <>
                <span className="dash-kicker-sep">·</span>
                {uptimeLabel}
              </>
            )}
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
        </div>
      </section>

      <div className="simple-instruments">
        <button
          type="button"
          className="instrument accent-cyan instrument-click simple-instrument"
          onClick={() => onGoServers?.()}
        >
          <header className="instrument-head">
            <span className="instrument-label">{t("dashboard.node")}</span>
            <span className="instrument-tag">
              {!nodeReady
                ? "…"
                : (node?.protocol?.toUpperCase() ?? "—")}
            </span>
          </header>
          <div className="instrument-value sm">
            {!nodeReady ? (
              <span className="skel skel-inline skel-w-50" aria-hidden />
            ) : (
              (node?.name ?? t("simple.pickNode"))
            )}
          </div>
          <div className="instrument-kv mono">
            <div>
              <span className="kv-k">{t("dashboard.latency")}</span>
              <span
                className={`kv-v lat ${testing ? "lat-none" : latencyClass(node?.latency_ms)}`}
              >
                {testing ? "…" : fmtLatency(node?.latency_ms)}
              </span>
            </div>
          </div>
        </button>
        <button
          type="button"
          className="instrument accent-blue instrument-click simple-instrument"
          onClick={() => onGoTraffic?.()}
        >
          <header className="instrument-head">
            <span className="instrument-label">{t("dashboard.cardTraffic")}</span>
            <span className="instrument-tag">NET</span>
          </header>
          <div className="instrument-traffic">
            <div>
              <span className="tr-dir down">↓</span> {fmtSpeed(down)}
            </div>
            <div>
              <span className="tr-dir up">↑</span> {fmtSpeed(up)}
            </div>
          </div>
          <div className="instrument-kv mono">
            <div>
              <span className="kv-k">Σ</span>
              <span className="kv-v">
                {fmtBytes((proxy?.upload_total ?? 0) + (proxy?.download_total ?? 0))}
              </span>
            </div>
          </div>
        </button>
      </div>

      <aside className="simple-rail" aria-label={t("dashboard.quickControls")}>
        <div className="dash-rail-title mono">{t("dashboard.quickControls")}</div>
        <div className="dash-inline-row">
          <span className="dash-inline-label">{t("dashboard.routing")}</span>
          <GlassSeg
            value={outboundMode}
            ready={!!proxy}
            ariaLabel={t("dashboard.routing")}
            disabled={busy || !proxy || customRuntime}
            onChange={(v) => void onSetMode(v as OutboundMode)}
            options={[
              { value: "rule", label: t("dashboard.modeRule") },
              { value: "global", label: t("dashboard.modeGlobal") },
              { value: "direct", label: t("dashboard.modeDirect") },
            ]}
          />
        </div>
        <div className="dash-inline-row">
          <span
            className={`dash-inline-label${smartProbing ? " dash-smart-probing" : ""}`}
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
            ready={!!proxy}
            ariaLabel={t("dashboard.autoSelect")}
            disabled={busy || !proxy || customRuntime}
            disabledValues={
              new Set(
                [
                  smartProbing ? "smart" : null,
                  nodeCount === 0 && autoSelectMode === "off" && !smartProbing
                    ? "kernel"
                    : null,
                  nodeCount === 0 && autoSelectMode === "off" && !smartProbing
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
        <div className="dash-inline-row">
          <span
            className={`dash-inline-label${captureBusy ? " dash-smart-probing" : ""}`}
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
            ready={!!proxy}
            ariaLabel={t("dashboard.capture")}
            disabled={!proxy}
            disabledValues={
              new Set(
                [
                  customRuntime || (nodeCount === 0 && captureMode !== "tun")
                    ? "tun"
                    : null,
                  customRuntime && !proxy?.custom_inbound_port ? "system" : null,
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

      <SimpleTrafficSpark
        samples={spark}
        up={up}
        down={down}
        conns={conns}
        running={running}
        label={t("simple.spark")}
        idleLabel={t("simple.sparkIdle")}
        idleConnsLabel={t("simple.sparkIdleConns")}
        connsLabel={t("simple.sparkConns", { n: conns })}
        onOpen={onGoTraffic}
      />
    </div>
  );
}

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
