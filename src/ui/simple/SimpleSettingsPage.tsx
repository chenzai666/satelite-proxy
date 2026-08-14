import { useCallback, useEffect, useRef, useState } from "react";
import {
  getSettings,
  listAllNodes,
  getProxyStatus,
  setOutboundMode,
  smartSwitchNow,
  updateSettings,
} from "../../api";
import { GlassSeg } from "../../components/GlassSeg";
import { SolidSelect } from "../../components/SolidSelect";
import { GlassSwitchControl } from "../../components/GlassSwitchControl";
import { useCaptureModeSwitch } from "../../hooks/useCaptureModeSwitch";
import { useI18n, type Locale } from "../../i18n";
import { useTheme } from "../../theme";
import type {
  AppSettings,
  AutoSelectMode,
  OutboundMode,
  ProxyStatus,
  ThemeId,
  TrayIconStyle,
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

  const onCaptureError = useCallback((msg: string) => {
    setError(msg);
  }, []);

  const { captureMode: captureResolved, captureBusy, requestCaptureMode } =
    useCaptureModeSwitch(proxy, setProxy, onCaptureError);

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
            <GlassSeg
              value={captureResolved}
              ariaLabel={t("dashboard.capture")}
              disabled={busy}
              disabledValues={
                new Set(
                  [
                    nodeCount === 0 && captureResolved !== "tun" ? "tun" : null,
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
            <GlassSeg
              value={autoSelectMode}
              ariaLabel="节点切换"
              disabled={busy}
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
              onChange={(v) => void onSetAutoSelect(v as AutoSelectMode)}
              options={[
                { value: "off", label: "手动" },
                { value: "kernel", label: "自动" },
                { value: "smart", label: "智能" },
              ]}
            />
          </div>
          <div className="simple-setting-row simple-setting-col">
            <div className="simple-setting-title">路由模式</div>
            <GlassSeg
              value={mode}
              ariaLabel="路由模式"
              disabled={busy}
              onChange={(v) =>
                void (async () => {
                  setBusy(true);
                  try {
                    setProxy(await setOutboundMode(v as OutboundMode));
                  } catch (e) {
                    setError(typeof e === "string" ? e : String(e));
                  } finally {
                    setBusy(false);
                  }
                })()
              }
              options={[
                { value: "rule", label: "规则" },
                { value: "global", label: "全局" },
                { value: "direct", label: "直连" },
              ]}
            />
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
            <GlassSwitchControl
              checked={!!settings?.launch_at_login}
              title="开机启动"
              disabled={busy || !settings}
              ready={!!settings}
              onChange={(next) =>
                void patchSettings({
                  launchAtLogin: next,
                })
              }
            />
          </div>
          <div className="simple-setting-row">
            <div>
              <div className="simple-setting-title">关窗到托盘</div>
            </div>
            <GlassSwitchControl
              checked={!!settings?.close_to_tray}
              title="关窗到托盘"
              disabled={busy || !settings}
              ready={!!settings}
              onChange={(next) =>
                void patchSettings({
                  closeToTray: next,
                })
              }
            />
          </div>
          <div className="simple-setting-row">
            <div>
              <div className="simple-setting-title">{t("settings.unloadUi")}</div>
              <div className="simple-setting-desc muted">
                {t("settings.unloadUiDesc")}
              </div>
            </div>
            <GlassSwitchControl
              checked={!!settings?.unload_ui_on_tray}
              title={t("settings.unloadUi")}
              disabled={busy || !settings}
              ready={!!settings}
              onChange={(next) =>
                void patchSettings({
                  unloadUiOnTray: next,
                })
              }
            />
          </div>
        </div>
      </section>

      <section className="simple-section">
        <div className="simple-section-label muted">外观</div>
        <div className="simple-card simple-settings-group">
          <div className="simple-setting-row simple-setting-col">
            <div className="simple-setting-title">{t("settings.theme")}</div>
            <GlassSeg
              value={theme}
              ariaLabel={t("settings.theme")}
              onChange={(v) => void setTheme(v as ThemeId)}
              options={[
                { value: "day", label: "Day" },
                { value: "aerospace", label: "Mission" },
              ]}
            />
          </div>
          <div className="simple-setting-row simple-setting-col">
            <div className="simple-setting-title">{t("settings.trayIcon")}</div>
            <SolidSelect
              className="solid-select-compact"
              aria-label={t("settings.trayIcon")}
              value={settings?.tray_icon ?? "badge"}
              disabled={busy || !settings}
              onChange={(v) => void patchSettings({ trayIcon: v as TrayIconStyle })}
              options={[
                { value: "badge", label: t("settings.trayIconBadge") },
                { value: "mark", label: t("settings.trayIconMark") },
                { value: "ghost", label: t("settings.trayIconGhost") },
                { value: "buddy", label: t("settings.trayIconBuddy") },
              ]}
            />
          </div>
          <div className="simple-setting-row simple-setting-col">
            <div className="simple-setting-title">语言</div>
            <GlassSeg
              value={locale}
              ariaLabel="语言"
              onChange={(v) => void setLocale(v as Locale)}
              options={[
                { value: "zh", label: "中文" },
                { value: "en", label: "EN" },
              ]}
            />
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
