import { useCallback, useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import {
  getProxyStatus,
  getSettings,
  listAllNodes,
  setCaptureMode,
  setOutboundMode,
  smartSwitchNow,
  updateSettings,
} from "../../api";
import { useI18n, type Locale } from "../../i18n";
import { useTheme } from "../../theme";
import type {
  AppSettings,
  AutoSelectMode,
  OutboundMode,
  ProxyStatus,
  ThemeId,
} from "../../types";
import { useUiMode } from "../UiModeContext";

export function SimpleSettingsPage() {
  const { t, locale, setLocale } = useI18n();
  const { theme, setTheme } = useTheme();
  const { setMode } = useUiMode();
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [proxy, setProxy] = useState<ProxyStatus | null>(null);
  const [nodeCount, setNodeCount] = useState(0);
  const [busy, setBusy] = useState(false);
  const [captureBusy, setCaptureBusy] = useState(false);
  const [captureUi, setCaptureUi] = useState<"off" | "system" | "tun" | null>(
    null,
  );
  const captureGenRef = useRef(0);
  const [smartProbing, setSmartProbing] = useState(false);
  const smartGenRef = useRef(0);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const [s, p, nodes] = await Promise.all([
        getSettings(),
        getProxyStatus().catch(() => null),
        listAllNodes().catch(() => []),
      ]);
      setSettings(s);
      setProxy(p);
      setNodeCount(nodes.length);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  async function patchSettings(partial: Parameters<typeof updateSettings>[0]) {
    setBusy(true);
    setError(null);
    try {
      const s = await updateSettings(partial);
      setSettings(s);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  function resolveAutoSelect(): AutoSelectMode {
    const raw =
      proxy?.auto_select ??
      settings?.auto_select ??
      (proxy?.smart_switch || settings?.smart_switch ? "smart" : "off");
    if (raw === "smart" || raw === "kernel") return raw;
    return "off";
  }

  async function onSetAutoSelect(mode: AutoSelectMode) {
    const prev = resolveAutoSelect();
    if (mode === prev) return;
    setError(null);
    if (mode !== "smart") {
      smartGenRef.current += 1;
      setSmartProbing(false);
    }
    setProxy((p) =>
      p ? { ...p, auto_select: mode, smart_switch: mode === "smart" } : p,
    );
    setSettings((s) =>
      s ? { ...s, auto_select: mode, smart_switch: mode === "smart" } : s,
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
            setError("请先启动代理，智能切换才能探测节点。");
          } else if (
            r.message === "all probes failed" ||
            r.message === "clash api unavailable"
          ) {
            setError("智能切换探测失败，请检查网络或节点。");
          } else if (r.message === "no nodes") {
            setError("没有可用节点，无法智能切换。");
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
        setSettings((s) =>
          s
            ? { ...s, auto_select: prev, smart_switch: prev === "smart" }
            : s,
        );
      }
    } finally {
      if (gen === smartGenRef.current) setSmartProbing(false);
    }
  }

  const mode = (proxy?.outbound_mode ?? "rule") as OutboundMode;
  const autoSelectMode = resolveAutoSelect();

  return (
    <div className="simple-page simple-settings">
      <header className="simple-page-head">
        <div>
          <div className="simple-kicker muted">APP</div>
          <h1 className="simple-title">设置</h1>
        </div>
      </header>

      {error && <div className="banner error">{error}</div>}

      <section className="simple-section">
        <div className="simple-section-label muted">连接</div>
        <div className="simple-card simple-settings-group">
          <div className="simple-setting-row simple-auto-select-row">
            <div>
              <div
                className={`simple-setting-title${captureBusy ? " dash-smart-probing" : ""}`}
              >
                {captureBusy ? (
                  <>
                    <span className="lat-spinner dash-smart-spinner" aria-hidden />
                    <span>{t("dashboard.captureSwitching")}</span>
                  </>
                ) : (
                  t("dashboard.capture")
                )}
              </div>
              <div className="muted simple-setting-desc">
                {t("dashboard.captureDesc")}
              </div>
            </div>
            <div
              className="segmented compact mode-seg simple-auto-seg"
              role="group"
              aria-label={t("dashboard.capture")}
              aria-busy={captureBusy}
            >
              <span
                className="seg-indicator"
                aria-hidden="true"
                style={{
                  transform: `translateX(${
                    (captureUi ??
                      (proxy?.tun_enabled
                        ? "tun"
                        : proxy?.system_proxy
                          ? "system"
                          : "off")) === "off"
                      ? 0
                      : (captureUi ??
                            (proxy?.tun_enabled
                              ? "tun"
                              : proxy?.system_proxy
                                ? "system"
                                : "off")) === "system"
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
              ).map(([key, label]) => {
                const resolved =
                  captureUi ??
                  (proxy?.tun_enabled
                    ? "tun"
                    : proxy?.system_proxy
                      ? "system"
                      : "off");
                const active = resolved === key;
                return (
                  <button
                    key={key}
                    type="button"
                    className={`seg ${active ? "active" : ""}`}
                    disabled={
                      busy ||
                      (captureBusy && !active) ||
                      (key === "tun" && nodeCount === 0 && resolved !== "tun")
                    }
                    title={
                      key === "tun"
                        ? t("dashboard.captureTunHint")
                        : key === "system"
                          ? t("dashboard.captureSystemHint")
                          : t("dashboard.captureDesc")
                    }
                    onClick={() => {
                      if (active || captureBusy) return;
                      const prev = proxy;
                      const gen = ++captureGenRef.current;
                      flushSync(() => {
                        setCaptureUi(key);
                        setCaptureBusy(true);
                        setError(null);
                        if (prev) {
                          setProxy({
                            ...prev,
                            system_proxy: key === "system",
                            tun_enabled: key === "tun",
                          });
                        }
                      });
                      void (async () => {
                        try {
                          const s = await setCaptureMode(key);
                          if (gen !== captureGenRef.current) return;
                          setProxy(s);
                          setCaptureUi(null);
                        } catch (e) {
                          if (gen !== captureGenRef.current) return;
                          setError(typeof e === "string" ? e : String(e));
                          setCaptureUi(null);
                          if (prev) {
                            setProxy(prev);
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
                    }}
                  >
                    {label}
                  </button>
                );
              })}
            </div>
          </div>
          <div className="simple-setting-row simple-auto-select-row">
            <div>
              <div className="simple-setting-title">
                {smartProbing ? "智能探测中…" : "节点切换"}
              </div>
              <div className="muted simple-setting-desc">
                {smartProbing
                  ? "正在探测节点，可切到「手动」结束"
                  : "手动 / 自动（urltest）/ 智能（应用）"}
              </div>
            </div>
            <div
              className="segmented compact mode-seg simple-auto-seg"
              role="group"
              aria-label="节点切换"
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
                  ["off", "手动"],
                  ["kernel", "自动"],
                  ["smart", "智能"],
                ] as const
              ).map(([key, label]) => (
                <button
                  key={key}
                  type="button"
                  className={`seg ${autoSelectMode === key ? "active" : ""}`}
                  disabled={
                    busy ||
                    (smartProbing && key === "smart") ||
                    (nodeCount === 0 &&
                      key !== "off" &&
                      autoSelectMode === "off" &&
                      !smartProbing)
                  }
                  onClick={() => void onSetAutoSelect(key)}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>
          <div className="simple-setting-row simple-setting-col">
            <div className="simple-setting-title">路由模式</div>
            <div className="segmented compact simple-seg-equal">
              {(
                [
                  ["rule", "规则"],
                  ["global", "全局"],
                  ["direct", "直连"],
                ] as const
              ).map(([k, label]) => (
                <button
                  key={k}
                  type="button"
                  className={`seg ${mode === k ? "active" : ""}`}
                  disabled={busy}
                  onClick={() =>
                    void (async () => {
                      setBusy(true);
                      try {
                        setProxy(await setOutboundMode(k));
                      } catch (e) {
                        setError(typeof e === "string" ? e : String(e));
                      } finally {
                        setBusy(false);
                      }
                    })()
                  }
                >
                  {label}
                </button>
              ))}
            </div>
          </div>
        </div>
      </section>

      <section className="simple-section">
        <div className="simple-section-label muted">窗口与启动</div>
        <div className="simple-card simple-settings-group">
          <div className="simple-setting-row">
            <div>
              <div className="simple-setting-title">开机启动</div>
            </div>
            <button
              type="button"
              role="switch"
              className={`switch ${settings?.launch_at_login ? "on" : ""}`}
              disabled={busy || !settings}
              aria-checked={!!settings?.launch_at_login}
              onClick={() =>
                void patchSettings({
                  launchAtLogin: !settings?.launch_at_login,
                })
              }
            >
              <span className="switch-thumb" />
            </button>
          </div>
          <div className="simple-setting-row">
            <div>
              <div className="simple-setting-title">关窗到托盘</div>
            </div>
            <button
              type="button"
              role="switch"
              className={`switch ${settings?.close_to_tray ? "on" : ""}`}
              disabled={busy || !settings}
              aria-checked={!!settings?.close_to_tray}
              onClick={() =>
                void patchSettings({
                  closeToTray: !settings?.close_to_tray,
                })
              }
            >
              <span className="switch-thumb" />
            </button>
          </div>
          <div className="simple-setting-row">
            <div>
              <div className="simple-setting-title">{t("settings.unloadUi")}</div>
              <div className="simple-setting-desc muted">
                {t("settings.unloadUiDesc")}
              </div>
            </div>
            <button
              type="button"
              role="switch"
              className={`switch ${settings?.unload_ui_on_tray ? "on" : ""}`}
              disabled={busy || !settings}
              aria-checked={!!settings?.unload_ui_on_tray}
              onClick={() =>
                void patchSettings({
                  unloadUiOnTray: !settings?.unload_ui_on_tray,
                })
              }
            >
              <span className="switch-thumb" />
            </button>
          </div>
        </div>
      </section>

      <section className="simple-section">
        <div className="simple-section-label muted">外观</div>
        <div className="simple-card simple-settings-group">
          <div className="simple-setting-row simple-setting-col">
            <div className="simple-setting-title">{t("settings.theme")}</div>
            <div className="segmented compact">
              <button
                type="button"
                className={`seg ${theme === "day" ? "active" : ""}`}
                onClick={() => void setTheme("day" as ThemeId)}
              >
                Day
              </button>
              <button
                type="button"
                className={`seg ${theme === "aerospace" ? "active" : ""}`}
                onClick={() => void setTheme("aerospace" as ThemeId)}
              >
                Mission
              </button>
            </div>
          </div>
          <div className="simple-setting-row simple-setting-col">
            <div className="simple-setting-title">语言</div>
            <div className="segmented compact">
              <button
                type="button"
                className={`seg ${locale === "zh" ? "active" : ""}`}
                onClick={() => void setLocale("zh" as Locale)}
              >
                中文
              </button>
              <button
                type="button"
                className={`seg ${locale === "en" ? "active" : ""}`}
                onClick={() => void setLocale("en" as Locale)}
              >
                EN
              </button>
            </div>
          </div>
        </div>
      </section>

      <section className="simple-section">
        <div className="simple-section-label muted">高级</div>
        <div className="simple-card simple-settings-group">
          <div className="simple-setting-row">
            <div>
              <div className="simple-setting-title">运行模式</div>
              <div className="muted simple-setting-desc">
                也可点顶部 ⋯ 切换 · 完整模式含规则 / DNS 等
              </div>
            </div>
          </div>
          <button
            type="button"
            className="simple-link-row"
            onClick={() => setMode("pro")}
          >
            <div>
              <div className="simple-setting-title">切换到完整模式</div>
              <div className="muted simple-setting-desc">
                规则、DNS、日志详情等专业功能
              </div>
            </div>
            <span className="muted">→</span>
          </button>
        </div>
      </section>
    </div>
  );
}
