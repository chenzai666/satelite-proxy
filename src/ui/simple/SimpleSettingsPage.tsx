import { useCallback, useEffect, useRef, useState } from "react";
import {
  getProxyStatus,
  getSettings,
  listAllNodes,
  setOutboundMode,
  setSystemProxy,
  setTunEnabled,
  smartSwitchNow,
  updateSettings,
} from "../../api";
import { useI18n, type Locale } from "../../i18n";
import { useTheme } from "../../theme";
import type { AppSettings, OutboundMode, ProxyStatus, ThemeId } from "../../types";
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

  async function onToggleSmartSwitch(next: boolean) {
    setError(null);
    if (!next) {
      smartGenRef.current += 1;
      setSmartProbing(false);
      setProxy((prev) => (prev ? { ...prev, smart_switch: false } : prev));
      setSettings((prev) =>
        prev ? { ...prev, smart_switch: false } : prev,
      );
      try {
        await updateSettings({ smartSwitch: false });
        const s = await getProxyStatus().catch(() => null);
        if (s) setProxy(s);
      } catch (e) {
        setError(typeof e === "string" ? e : String(e));
      }
      return;
    }

    const gen = ++smartGenRef.current;
    setSmartProbing(true);
    setProxy((prev) => (prev ? { ...prev, smart_switch: true } : prev));
    setSettings((prev) => (prev ? { ...prev, smart_switch: true } : prev));
    try {
      await updateSettings({ smartSwitch: true });
      if (gen !== smartGenRef.current) {
        await updateSettings({ smartSwitch: false }).catch(() => {});
        return;
      }
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
      if (gen !== smartGenRef.current) return;
      await reload();
      setProxy((prev) => (prev ? { ...prev, smart_switch: true } : prev));
    } catch (e) {
      if (gen === smartGenRef.current) {
        setError(typeof e === "string" ? e : String(e));
        setProxy((prev) =>
          prev ? { ...prev, smart_switch: false } : prev,
        );
        setSettings((prev) =>
          prev ? { ...prev, smart_switch: false } : prev,
        );
      }
    } finally {
      if (gen === smartGenRef.current) setSmartProbing(false);
    }
  }

  const mode = (proxy?.outbound_mode ?? "rule") as OutboundMode;
  const smartOn =
    proxy?.smart_switch ?? settings?.smart_switch ?? false;

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
          <div className="simple-setting-row">
            <div>
              <div className="simple-setting-title">系统代理</div>
              <div className="muted simple-setting-desc">
                HTTP/SOCKS 指向本地 mixed 端口
              </div>
            </div>
            <button
              type="button"
              role="switch"
              className={`switch ${proxy?.system_proxy ? "on" : ""}`}
              disabled={busy || !proxy?.running}
              aria-checked={!!proxy?.system_proxy}
              onClick={() =>
                void (async () => {
                  setBusy(true);
                  try {
                    setProxy(
                      await setSystemProxy(!(proxy?.system_proxy ?? false)),
                    );
                  } catch (e) {
                    setError(typeof e === "string" ? e : String(e));
                  } finally {
                    setBusy(false);
                  }
                })()
              }
            >
              <span className="switch-thumb" />
            </button>
          </div>
          <div className="simple-setting-row">
            <div>
              <div className="simple-setting-title">TUN 模式</div>
              <div className="muted simple-setting-desc">
                全局接管（可能需要管理员密码）
              </div>
            </div>
            <button
              type="button"
              role="switch"
              className={`switch ${proxy?.tun_enabled ? "on" : ""}`}
              disabled={busy}
              aria-checked={!!proxy?.tun_enabled}
              onClick={() =>
                void (async () => {
                  setBusy(true);
                  try {
                    setProxy(
                      await setTunEnabled(!(proxy?.tun_enabled ?? false)),
                    );
                  } catch (e) {
                    setError(typeof e === "string" ? e : String(e));
                  } finally {
                    setBusy(false);
                  }
                })()
              }
            >
              <span className="switch-thumb" />
            </button>
          </div>
          <div className="simple-setting-row">
            <div>
              <div className="simple-setting-title">
                {smartProbing ? "智能探测中…" : "智能切换"}
              </div>
              <div className="muted simple-setting-desc">
                {smartProbing
                  ? "正在探测节点，可关闭以结束"
                  : "开启后探测并自动选最佳节点"}
              </div>
            </div>
            <button
              type="button"
              role="switch"
              className={`switch ${smartOn ? "on" : ""}`}
              disabled={busy || (nodeCount === 0 && !smartOn)}
              aria-checked={smartOn}
              aria-busy={smartProbing}
              onClick={() => void onToggleSmartSwitch(!smartOn)}
            >
              <span className="switch-thumb" />
            </button>
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
