import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  checkCoreUpdate,
  downloadCore,
  getCoreInfo,
  getProxyStatus,
  getSettings,
  restartProxy,
  updateSettings,
} from "../api";
import { GlassButton } from "../components/GlassButton";
import { SolidSelect } from "../components/SolidSelect";
import { GlassSeg } from "../components/GlassSeg";
import { GlassSwitchControl } from "../components/GlassSwitchControl";
import { TrayIconPicker } from "../components/TrayIconPicker";
import { useI18n, type Locale, type MessageKey } from "../i18n";
import { ACCENTS } from "../theme/accents";
import { useTheme } from "../theme";
import type {
  AppSettings,
  CoreDownloadProgress,
  CoreInfo,
  HeroStyle,
  ThemeId,
} from "../types";
import { RulesPage } from "./RulesPage";
import { DnsPage } from "./DnsPage";
import { HostsPage } from "./HostsPage";

type SettingsTab = "app" | "rules" | "dns" | "hosts" | "core";

const CUSTOM_BLOCKED_TABS = new Set(["rules", "dns", "hosts"]);

// Accent preset names are picked from the i18n catalog rather than
// AccentPreset.name (theme/accents.ts), which is display data only and not
// locale-aware.
const ACCENT_LABEL_KEY: Record<string, MessageKey> = {
  green: "accent.green",
  blue: "accent.blue",
  purple: "accent.purple",
  pink: "accent.pink",
  orange: "accent.orange",
  cyan: "accent.cyan",
};

function fmtCoreBytes(value: number) {
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

export function SettingsPage() {
  const { t, locale, setLocale } = useI18n();
  const { theme, setTheme, accent, setAccent, heroStyle, setHeroStyle } =
    useTheme();
  const [tab, setTab] = useState<SettingsTab>("app");
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [mixed, setMixed] = useState("2080");
  const [api, setApi] = useState("19090");
  const [probe, setProbe] = useState("");
  const [tunStack, setTunStack] = useState("mixed");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [core, setCore] = useState<CoreInfo | null>(null);
  const [coreBusy, setCoreBusy] = useState(false);
  const [coreChecking, setCoreChecking] = useState(false);
  const [coreError, setCoreError] = useState<string | null>(null);
  const [coreProxyAvailable, setCoreProxyAvailable] = useState(false);
  const [coreProgress, setCoreProgress] =
    useState<CoreDownloadProgress | null>(null);

  const tabs = useMemo(
    () =>
      [
        {
          id: "app" as const,
          label: t("settings.tabApp"),
          hint: t("settings.hintApp"),
        },
        {
          id: "rules" as const,
          label: t("settings.tabRules"),
          hint: t("settings.hintRules"),
        },
        {
          id: "dns" as const,
          label: t("settings.tabDns"),
          hint: t("settings.hintDns"),
        },
        {
          id: "hosts" as const,
          label: t("settings.tabHosts"),
          hint: t("settings.hintHosts"),
        },
        {
          id: "core" as const,
          label: t("settings.tabCore"),
          hint: t("settings.hintCore"),
        },
      ] as const,
    [t],
  );

  const runCoreUpdateCheck = useCallback(
    async (localVersion: string | null, reportError: boolean) => {
      setCoreChecking(true);
      if (reportError) setCoreError(null);
      try {
        const update = await checkCoreUpdate(localVersion);
        setCore((prev) =>
          prev
            ? {
                ...prev,
                latest_version: update.latest_version,
                update_available: update.update_available,
              }
            : prev,
        );
      } catch (e) {
        if (reportError) {
          setCoreError(typeof e === "string" ? e : String(e));
        }
      } finally {
        setCoreChecking(false);
      }
    },
    [],
  );

  const reloadCore = useCallback(async () => {
    setCoreError(null);
    try {
      const local = await getCoreInfo();
      setCore(local);
      void runCoreUpdateCheck(local.version ?? null, false);
    } catch (e) {
      setCoreError(typeof e === "string" ? e : String(e));
    }
  }, [runCoreUpdateCheck]);

  useEffect(() => {
    getSettings()
      .then((s) => {
        setSettings(s);
        setMixed(String(s.mixed_port));
        setApi(String(s.api_port));
        setProbe(s.probe_url);
        setTunStack(s.tun_stack || "mixed");
      })
      .catch((e) => setError(typeof e === "string" ? e : String(e)));
    void reloadCore();
  }, [reloadCore]);

  useEffect(() => {
    if (tab !== "core") return;
    void getProxyStatus()
      .then((status) => setCoreProxyAvailable(status.running))
      .catch(() => setCoreProxyAvailable(false));
  }, [tab]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<CoreDownloadProgress>("core-download-progress", (event) => {
      setCoreProgress(event.payload);
      setCoreProxyAvailable(event.payload.via_proxy);
    }).then((dispose) => {
      unlisten = dispose;
    });
    return () => unlisten?.();
  }, []);

  async function onSaveNetwork() {
    setBusy(true);
    setError(null);
    try {
      const mixedPort = Number(mixed);
      const apiPort = Number(api);
      if (!Number.isFinite(mixedPort) || mixedPort < 1 || mixedPort > 65535) {
        throw new Error(t("settings.invalidMixed"));
      }
      if (!Number.isFinite(apiPort) || apiPort < 1 || apiPort > 65535) {
        throw new Error(t("settings.invalidApi"));
      }
      const s = await updateSettings({
        mixedPort,
        apiPort,
        probeUrl: probe.trim() || null,
        tunStack: tunStack.trim() || "mixed",
      });
      setSettings(s);
      // These options are consumed when sing-box starts; apply them together.
      const status = await getProxyStatus().catch(() => null);
      if (status?.running) {
        await restartProxy();
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onDownloadCore() {
    setCoreBusy(true);
    setCoreError(null);
    const status = await getProxyStatus().catch(() => null);
    const viaProxy = !!status?.running;
    setCoreProxyAvailable(viaProxy);
    setCoreProgress({
      stage: "preparing",
      downloaded: 0,
      total: null,
      percent: null,
      via_proxy: viaProxy,
    });
    try {
      await downloadCore(null);
      await reloadCore();
    } catch (e) {
      setCoreError(typeof e === "string" ? e : String(e));
    } finally {
      setCoreBusy(false);
    }
  }

  async function onCheckCoreUpdate() {
    await runCoreUpdateCheck(core?.version ?? null, true);
  }

  async function patchApp(partial: Parameters<typeof updateSettings>[0]) {
    setError(null);
    try {
      const s = await updateSettings(partial);
      setSettings(s);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      try {
        const s = await getSettings();
        setSettings(s);
      } catch {
        /* ignore */
      }
    }
  }

  async function onChangeLocale(next: Locale) {
    if (next === locale) return;
    setError(null);
    try {
      await setLocale(next);
      const s = await getSettings();
      setSettings(s);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }

  async function onChangeTheme(next: ThemeId) {
    if (next === theme) return;
    setError(null);
    try {
      await setTheme(next);
      const s = await getSettings();
      setSettings(s);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }

  const customRuntime = (settings?.runtime_source ?? "").startsWith("singbox:");

  useEffect(() => {
    if (customRuntime && CUSTOM_BLOCKED_TABS.has(tab)) {
      setTab("app");
    }
  }, [customRuntime, tab]);

  const visibleTab =
    customRuntime && CUSTOM_BLOCKED_TABS.has(tab) ? "app" : tab;

  const needsSettings = visibleTab === "app" || visibleTab === "core";
  if (needsSettings && !settings && !error) {
    return <div className="page empty">{t("common.loading")}</div>;
  }

  const activeTab = tabs.find((x) => x.id === visibleTab)!;

  return (
    <div className="page settings-page settings-wide">
      <header className="page-header">
        <div>
          <h1>{t("settings.title")}</h1>
          <p className="page-desc">{activeTab.hint}</p>
        </div>
      </header>

      <GlassSeg
        value={visibleTab}
        ariaLabel="Settings sections"
        onChange={(v) => {
          if (customRuntime && CUSTOM_BLOCKED_TABS.has(v)) return;
          setTab(v as SettingsTab);
          setError(null);
        }}
        disabledValues={customRuntime ? CUSTOM_BLOCKED_TABS : undefined}
        titles={
          customRuntime
            ? {
                rules: t("config.customDisabled"),
                dns: t("config.customDisabled"),
                hosts: t("config.customDisabled"),
              }
            : undefined
        }
        options={tabs.map((x) => ({ value: x.id, label: x.label }))}
      />

      {error &&
        visibleTab !== "rules" &&
        visibleTab !== "dns" &&
        visibleTab !== "hosts" && (
        <div className="banner error">{error}</div>
      )}

      {/* key={tab} remounts on tab switch → triggers the page-enter fade/slide. */}
      <div
        className={`page-enter${visibleTab === "app" ? " settings-app-network-page" : ""}`}
        key={visibleTab}
      >
        {!customRuntime && visibleTab === "rules" && <RulesPage embedded />}

        {!customRuntime && visibleTab === "dns" && (
          <DnsPage embedded section="settings" />
        )}
        {!customRuntime && visibleTab === "hosts" && <HostsPage embedded />}
      {visibleTab === "app" && settings && (
        <section className="settings-panel" aria-label="Application">
          <div className="card settings-app-card">
            <div className="settings-app-prefs">
              <div className="settings-app-row settings-app-pref">
                <div className="settings-app-text">
                  <div className="settings-app-title">{t("settings.language")}</div>
                  <div className="settings-app-desc muted">
                    {t("settings.languageDesc")}
                  </div>
                </div>
                <GlassSeg
                  value={locale}
                  ariaLabel={t("settings.language")}
                  disabled={busy}
                  onChange={(v) => void onChangeLocale(v as Locale)}
                  options={[
                    { value: "zh", label: t("settings.langZh") },
                    { value: "en", label: t("settings.langEn") },
                  ]}
                />
              </div>
              <div className="settings-app-row settings-app-pref">
                <div className="settings-app-text">
                  <div className="settings-app-title">{t("settings.theme")}</div>
                  <div className="settings-app-desc muted">
                    {t("settings.themeDesc")}
                  </div>
                </div>
                <GlassSeg
                  value={theme}
                  ariaLabel={t("settings.theme")}
                  disabled={busy}
                  onChange={(v) => void onChangeTheme(v as ThemeId)}
                  options={[
                    { value: "aerospace", label: t("settings.themeAerospace") },
                    { value: "day", label: t("settings.themeDay") },
                  ]}
                />
              </div>
              <div className="settings-app-row settings-app-pref settings-accent-row">
                <div className="settings-app-text">
                  <div className="settings-app-title">{t("settings.accent")}</div>
                  <div className="settings-app-desc muted">
                    {t("settings.accentDesc")}
                  </div>
                </div>
                <div
                  className="settings-accent-swatches"
                  role="group"
                  aria-label={t("settings.accent")}
                >
                  {ACCENTS.map((a) => (
                    <button
                      key={a.id}
                      type="button"
                      className={`settings-accent-dot ${accent === a.id ? "active" : ""}`}
                      style={{ background: a[theme], color: a[theme] }}
                      title={t(ACCENT_LABEL_KEY[a.id] ?? "settings.accent")}
                      aria-label={t(ACCENT_LABEL_KEY[a.id] ?? "settings.accent")}
                      aria-pressed={accent === a.id}
                      disabled={busy}
                      onClick={() => void setAccent(a.id)}
                    >
                      {accent === a.id ? (
                        <span className="settings-accent-check">✓</span>
                      ) : (
                        ""
                      )}
                    </button>
                  ))}
                </div>
              </div>
              <div className="settings-app-row settings-app-pref settings-hero-row">
                <div className="settings-app-text">
                  <div className="settings-app-title">{t("settings.heroStyle")}</div>
                  <div className="settings-app-desc muted">
                    {t("settings.heroStyleDesc")}
                  </div>
                </div>
                <GlassSeg
                  value={heroStyle}
                  ariaLabel={t("settings.heroStyle")}
                  disabled={busy}
                  onChange={(v) => void setHeroStyle(v as HeroStyle)}
                  options={[
                    { value: "particle", label: t("settings.heroStyleParticle") },
                    { value: "classic", label: t("settings.heroStyleClassic") },
                  ]}
                />
              </div>
              <div className="settings-app-row settings-app-pref settings-tray-icon-row">
                <div className="settings-app-text">
                  <div className="settings-app-title">{t("settings.trayIcon")}</div>
                  <div className="settings-app-desc muted">
                    {t("settings.trayIconDesc")}
                  </div>
                </div>
                <TrayIconPicker
                  value={settings?.tray_icon}
                  disabled={busy}
                  aria-label={t("settings.trayIcon")}
                  onChange={(v) => void patchApp({ trayIcon: v })}
                />
              </div>
            </div>
            <div className="settings-app-toggles">
              <AppToggle
                title={t("settings.closeToTray")}
                desc={t("settings.closeToTrayDesc")}
                checked={settings?.close_to_tray !== false}
                disabled={busy}
                onChange={(v) => void patchApp({ closeToTray: v })}
              />
              <AppToggle
                title={t("settings.unloadUi")}
                desc={t("settings.unloadUiDesc")}
                checked={!!settings?.unload_ui_on_tray}
                disabled={busy}
                onChange={(v) => void patchApp({ unloadUiOnTray: v })}
              />
              <AppToggle
                title={t("settings.launchAtLogin")}
                desc={t("settings.launchAtLoginDesc")}
                checked={!!settings?.launch_at_login}
                disabled={busy}
                onChange={(v) => void patchApp({ launchAtLogin: v })}
              />
              <AppToggle
                title={t("settings.silentStart")}
                desc={t("settings.silentStartDesc")}
                checked={!!settings?.silent_start}
                disabled={busy}
                onChange={(v) => void patchApp({ silentStart: v })}
              />
              <AppToggle
                title={t("settings.autoStartProxy")}
                desc={t("settings.autoStartProxyDesc")}
                checked={!!settings?.auto_start_proxy}
                disabled={busy}
                onChange={(v) => void patchApp({ autoStartProxy: v })}
              />
              <AppToggle
                title={t("settings.closeOnSwitch")}
                desc={t("settings.closeOnSwitchDesc")}
                checked={settings?.close_connections_on_switch !== false}
                disabled={busy || (settings?.runtime_source ?? "").startsWith("singbox:")}
                onChange={(v) => void patchApp({ closeConnectionsOnSwitch: v })}
              />
              <AppToggle
                title={t("settings.findProcess")}
                desc={t("settings.findProcessDesc")}
                checked={settings?.find_process !== false}
                disabled={busy || (settings?.runtime_source ?? "").startsWith("singbox:")}
                onChange={(v) => void patchApp({ findProcess: v })}
              />
            </div>
          </div>
          <p className="settings-panel-note muted">{t("settings.toggleSaveNote")}</p>
        </section>
      )}

      {visibleTab === "app" && settings && (
        <section className="settings-panel" aria-label="Network">
          <div className="card settings-form settings-form-grid">
            <div className="settings-network-card-head field-span-2">
              <div>
                <strong>{t("settings.networkOptions")}</strong>
                <div className="muted">{t("settings.networkSaveNote")}</div>
              </div>
              <GlassButton
                variant="primary"
                icon="↻"
                disabled={busy || (settings?.runtime_source ?? "").startsWith("singbox:")}
                onClick={() => void onSaveNetwork()}
                title={t("settings.saveRestartCore")}
              >
                {busy ? t("common.saving") : t("settings.saveRestartCore")}
              </GlassButton>
            </div>
            <label className="field">
              <span>{t("settings.mixedPort")}</span>
              <input
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
                className="mono"
                value={mixed}
                disabled={(settings?.runtime_source ?? "").startsWith("singbox:")}
                onChange={(e) => setMixed(e.target.value)}
              />
            </label>
            <label className="field">
              <span>{t("settings.apiPort")}</span>
              <input
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
                className="mono"
                value={api}
                disabled={(settings?.runtime_source ?? "").startsWith("singbox:")}
                onChange={(e) => setApi(e.target.value)}
              />
            </label>
            <label className="field field-span-2">
              <span>{t("settings.probeUrl")}</span>
              <input
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
                className="mono"
                value={probe}
                onChange={(e) => setProbe(e.target.value)}
                placeholder="https://…"
              />
            </label>
            <div className="field field-span-2">
              <span>{t("settings.tunStack")}</span>
              <SolidSelect
                value={tunStack}
                onChange={setTunStack}
                aria-label={t("settings.tunStack")}
                options={[
                  { value: "mixed", label: "mixed" },
                  { value: "system", label: "system" },
                  { value: "gvisor", label: "gvisor" },
                ]}
              />
              <span className="field-hint muted">
                {t("settings.tunStackHint")}{" "}
                <span className="mono">
                  {settings?.tun_enabled
                    ? t("common.enabled")
                    : t("common.disabled")}
                </span>
              </span>
            </div>
          </div>
        </section>
      )}

      {visibleTab === "core" && (
        <section className="settings-panel" aria-label="Core">
          {coreError && <div className="banner error">{coreError}</div>}

          <div className="card core-card">
            <div className="core-row">
              <div>
                <div className="stat-label">{t("settings.coreStatus")}</div>
                <div className="core-status">
                  {core?.installed ? (
                    <span className="pill ok">
                      {core.source === "bundled"
                        ? t("settings.coreBundled")
                        : t("settings.coreInstalled")}
                    </span>
                  ) : (
                    <span className="pill warn">{t("settings.coreMissing")}</span>
                  )}
                  <span className="muted mono">{core?.platform ?? "…"}</span>
                </div>
              </div>
              <div className="core-actions">
                <GlassButton
                  icon="↻"
                  disabled={coreBusy || coreChecking || !core}
                  onClick={() => void onCheckCoreUpdate()}
                >
                  {coreChecking
                    ? t("settings.coreChecking")
                    : t("settings.coreCheck")}
                </GlassButton>
                <GlassButton
                  variant="primary"
                  icon="⤓"
                  disabled={coreBusy || coreChecking}
                  onClick={() => void onDownloadCore()}
                >
                  {coreBusy
                    ? t("settings.coreDownloading")
                    : core?.source === "downloaded"
                      ? core.update_available
                        ? t("settings.coreUpdate")
                        : t("settings.coreRedownload")
                      : t("settings.coreDownload")}
                </GlassButton>
              </div>
            </div>

            <div className="core-meta">
              <div>
                <span className="stat-label">{t("settings.coreCurrent")}</span>
                <div className="mono">
                  {core?.version ?? "—"}
                  {core?.source === "bundled" ? (
                    <span className="pill ok" style={{ marginLeft: 8 }}>
                      {t("settings.coreBundled")}
                    </span>
                  ) : null}
                  {core?.source === "downloaded" ? (
                    <span className="pill" style={{ marginLeft: 8 }}>
                      {t("settings.coreUser")}
                    </span>
                  ) : null}
                </div>
              </div>
              <div>
                <span className="stat-label">
                  {t("settings.coreBundledLatest")}
                </span>
                <div className="mono">
                  {core?.bundled_version ?? "—"} / {core?.latest_version ?? "—"}
                  {core?.update_available ? (
                    <span className="pill warn" style={{ marginLeft: 8 }}>
                      {t("settings.coreUpdateAvail")}
                    </span>
                  ) : null}
                </div>
              </div>
            </div>

            {core?.path && (
              <div className="core-path">
                <span className="stat-label">{t("settings.corePath")}</span>
                <code className="path-text mono">{core.path}</code>
              </div>
            )}

            <div
              className={`core-download-route ${coreProxyAvailable ? "via-proxy" : "direct"}`}
            >
              <span className="core-route-dot" aria-hidden />
              <span>
                {coreProxyAvailable
                  ? t("settings.coreProxyRoute")
                  : t("settings.coreDirectRoute")}
              </span>
            </div>

            {coreBusy && coreProgress && (
              <div className="core-download-progress" aria-live="polite">
                <div className="core-download-progress-head">
                  <span className="lat-spinner" aria-hidden />
                  <span>
                    {coreProgress.stage === "preparing"
                      ? t("settings.corePreparing")
                      : coreProgress.stage === "installing"
                        ? t("settings.coreInstalling")
                        : t("settings.coreDownloading")}
                  </span>
                  <span className="mono core-download-percent">
                    {coreProgress.percent != null
                      ? `${coreProgress.percent}%`
                      : "…"}
                  </span>
                </div>
                <div
                  className={`core-progress-track${coreProgress.percent == null ? " indeterminate" : ""}`}
                >
                  <span
                    style={{
                      width: `${coreProgress.percent ?? 24}%`,
                    }}
                  />
                </div>
                {coreProgress.downloaded > 0 && (
                  <div className="muted mono core-download-bytes">
                    {fmtCoreBytes(coreProgress.downloaded)}
                    {coreProgress.total
                      ? ` / ${fmtCoreBytes(coreProgress.total)}`
                      : ""}
                  </div>
                )}
              </div>
            )}

            <p className="hint">{t("settings.coreHint")}</p>
          </div>
        </section>
      )}
      </div>
    </div>
  );
}

function AppToggle({
  title,
  desc,
  checked,
  disabled,
  onChange,
}: {
  title: string;
  desc: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="settings-app-row">
      <div className="settings-app-text">
        <div className="settings-app-title">{title}</div>
        <div className="settings-app-desc muted">{desc}</div>
      </div>
      <GlassSwitchControl
        checked={checked}
        title={title}
        disabled={disabled}
        onChange={onChange}
      />
    </div>
  );
}
