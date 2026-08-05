import { useCallback, useEffect, useMemo, useState } from "react";
import {
  checkCoreUpdate,
  downloadCore,
  getCoreInfo,
  getProxyStatus,
  getSettings,
  restartProxy,
  updateSettings,
} from "../api";
import { SolidSelect } from "../components/SolidSelect";
import { useI18n, type Locale } from "../i18n";
import { useTheme } from "../theme";
import type { AppSettings, CoreInfo, ThemeId } from "../types";
import { RulesPage } from "./RulesPage";
import { DnsPage } from "./DnsPage";

type SettingsTab = "app" | "rules" | "dns" | "network" | "core";

export function SettingsPage() {
  const { t, locale, setLocale } = useI18n();
  const { theme, setTheme } = useTheme();
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
  const [coreError, setCoreError] = useState<string | null>(null);

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
          id: "network" as const,
          label: t("settings.tabNetwork"),
          hint: t("settings.hintNetwork"),
        },
        {
          id: "core" as const,
          label: t("settings.tabCore"),
          hint: t("settings.hintCore"),
        },
      ] as const,
    [t],
  );

  const reloadCore = useCallback(async () => {
    setCoreError(null);
    try {
      const local = await getCoreInfo();
      setCore(local);
      void checkCoreUpdate(local.version)
        .then((u) => {
          setCore((prev) =>
            prev
              ? {
                  ...prev,
                  latest_version: u.latest_version,
                  update_available: u.update_available,
                }
              : prev,
          );
        })
        .catch(() => {
          /* ignore */
        });
    } catch (e) {
      setCoreError(typeof e === "string" ? e : String(e));
    }
  }, []);

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
      const portsChanged =
        settings != null &&
        (settings.mixed_port !== mixedPort || settings.api_port !== apiPort);
      const s = await updateSettings({
        mixedPort,
        apiPort,
        probeUrl: probe.trim() || null,
        tunStack: tunStack.trim() || "mixed",
      });
      setSettings(s);
      // Port binds at core start — restart running core so new ports take effect.
      if (portsChanged) {
        const status = await getProxyStatus().catch(() => null);
        if (status?.running) {
          await restartProxy();
        }
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
    try {
      await downloadCore(null);
      await reloadCore();
    } catch (e) {
      setCoreError(typeof e === "string" ? e : String(e));
    } finally {
      setCoreBusy(false);
    }
  }

  async function patchApp(partial: Parameters<typeof updateSettings>[0]) {
    setBusy(true);
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
    } finally {
      setBusy(false);
    }
  }

  async function onChangeLocale(next: Locale) {
    if (next === locale) return;
    setBusy(true);
    setError(null);
    try {
      await setLocale(next);
      const s = await getSettings();
      setSettings(s);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onChangeTheme(next: ThemeId) {
    if (next === theme) return;
    setBusy(true);
    setError(null);
    try {
      await setTheme(next);
      const s = await getSettings();
      setSettings(s);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  const needsSettings = tab === "app" || tab === "network" || tab === "core";
  if (needsSettings && !settings && !error) {
    return <div className="page empty">{t("common.loading")}</div>;
  }

  const activeTab = tabs.find((x) => x.id === tab)!;

  return (
    <div className="page settings-page settings-wide">
      <header className="page-header">
        <div>
          <h1>{t("settings.title")}</h1>
          <p className="page-desc">{activeTab.hint}</p>
        </div>
        {tab === "network" && (
          <button type="button" disabled={busy} onClick={() => void onSaveNetwork()}>
            {busy ? t("common.saving") : t("common.save")}
          </button>
        )}
      </header>

      <div
        className="settings-tabs segmented compact"
        role="tablist"
        aria-label="Settings sections"
      >
        {tabs.map((x) => (
          <button
            key={x.id}
            type="button"
            role="tab"
            aria-selected={tab === x.id}
            className={`seg ${tab === x.id ? "active" : ""}`}
            onClick={() => {
              setTab(x.id);
              setError(null);
            }}
          >
            {x.label}
          </button>
        ))}
      </div>

      {error && tab !== "rules" && tab !== "dns" && (
        <div className="banner error">{error}</div>
      )}

      {tab === "rules" && <RulesPage embedded />}

      {tab === "dns" && <DnsPage embedded />}

      {tab === "app" && settings && (
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
                <div
                  className="segmented compact"
                  role="group"
                  aria-label={t("settings.language")}
                >
                  <button
                    type="button"
                    className={`seg ${locale === "zh" ? "active" : ""}`}
                    disabled={busy}
                    onClick={() => void onChangeLocale("zh")}
                  >
                    {t("settings.langZh")}
                  </button>
                  <button
                    type="button"
                    className={`seg ${locale === "en" ? "active" : ""}`}
                    disabled={busy}
                    onClick={() => void onChangeLocale("en")}
                  >
                    {t("settings.langEn")}
                  </button>
                </div>
              </div>
              <div className="settings-app-row settings-app-pref">
                <div className="settings-app-text">
                  <div className="settings-app-title">{t("settings.theme")}</div>
                  <div className="settings-app-desc muted">
                    {t("settings.themeDesc")}
                  </div>
                </div>
                <div
                  className="segmented compact"
                  role="group"
                  aria-label={t("settings.theme")}
                >
                  <button
                    type="button"
                    className={`seg ${theme === "aerospace" ? "active" : ""}`}
                    disabled={busy}
                    onClick={() => void onChangeTheme("aerospace")}
                  >
                    {t("settings.themeAerospace")}
                  </button>
                  <button
                    type="button"
                    className={`seg ${theme === "day" ? "active" : ""}`}
                    disabled={busy}
                    onClick={() => void onChangeTheme("day")}
                  >
                    {t("settings.themeDay")}
                  </button>
                </div>
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
                checked={!!settings?.close_connections_on_switch}
                disabled={busy}
                onChange={(v) => void patchApp({ closeConnectionsOnSwitch: v })}
              />
            </div>
          </div>
          <p className="settings-panel-note muted">{t("settings.toggleSaveNote")}</p>
        </section>
      )}

      {tab === "network" && settings && (
        <section className="settings-panel" aria-label="Network">
          <div className="card settings-form settings-form-grid">
            <label className="field">
              <span>{t("settings.mixedPort")}</span>
              <input
                className="mono"
                value={mixed}
                onChange={(e) => setMixed(e.target.value)}
              />
            </label>
            <label className="field">
              <span>{t("settings.apiPort")}</span>
              <input
                className="mono"
                value={api}
                onChange={(e) => setApi(e.target.value)}
              />
            </label>
            <label className="field field-span-2">
              <span>{t("settings.probeUrl")}</span>
              <input
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
          <p className="settings-panel-note muted">
            {t("settings.networkSaveNote")}
          </p>
        </section>
      )}

      {tab === "core" && (
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
              <button
                type="button"
                disabled={coreBusy}
                onClick={() => void onDownloadCore()}
              >
                {coreBusy
                  ? t("settings.coreDownloading")
                  : core?.source === "downloaded"
                    ? core.update_available
                      ? t("settings.coreUpdate")
                      : t("settings.coreRedownload")
                    : t("settings.coreDownload")}
              </button>
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

            <p className="hint">{t("settings.coreHint")}</p>
          </div>
        </section>
      )}
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
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        className={`switch ${checked ? "on" : ""}`}
        disabled={disabled}
        onClick={() => onChange(!checked)}
      >
        <span className="switch-thumb" />
      </button>
    </div>
  );
}
