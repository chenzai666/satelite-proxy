import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { listen } from "@tauri-apps/api/event";
import {
  checkCoreUpdate,
  diagnoseNetwork,
  downloadCore,
  getCoreInfo,
  getProxyStatus,
  getSettings,
  regenerateApiSecret,
  restartProxy,
  updateSettings,
} from "../api";
import { GlassButton } from "../components/GlassButton";
import { SolidSelect } from "../components/SolidSelect";
import { GlassSeg } from "../components/GlassSeg";
import { GlassSwitchControl } from "../components/GlassSwitchControl";
import { TrayIconPicker } from "../components/TrayIconPicker";
import { DecryptReveal } from "../components/DecryptReveal";
import buymecoffeeUrl from "../assets/buymecoffee.png";
import { useI18n, type Locale, type MessageKey } from "../i18n";
import { ACCENTS } from "../theme/accents";
import { useTheme } from "../theme";
import type {
  AppSettings,
  CoreDownloadProgress,
  CoreInfo,
  DiagnosticIssue,
  ExtraInbound,
  HeroStyle,
  ThemeId,
} from "../types";
import { RulesPage } from "./RulesPage";
import { DnsPage } from "./DnsPage";
import { HostsPage } from "./HostsPage";

type SettingsTab = "app" | "ports" | "rules" | "dns" | "hosts" | "core";

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
  /** Sponsor QR panel (decrypt-reveal over the image). */
  const [sponsorOpen, setSponsorOpen] = useState(false);

  // Click outside the panel/link dismisses it. Ignore clicks inside either
  // node: the link is portaled to <body>, so React stopPropagation does not
  // reliably reach this native document listener.
  useEffect(() => {
    if (!sponsorOpen) return;
    const close = (e: MouseEvent) => {
      const node = e.target;
      if (
        node instanceof Element &&
        node.closest(".sponsor-panel, .sponsor-link")
      ) {
        return;
      }
      setSponsorOpen(false);
    };
    document.addEventListener("click", close);
    return () => document.removeEventListener("click", close);
  }, [sponsorOpen]);
  const [mixed, setMixed] = useState("2080");
  /** Main mixed inbound listens on 0.0.0.0 (LAN) instead of 127.0.0.1. */
  const [allowLan, setAllowLan] = useState(false);
  const [api, setApi] = useState("19090");
  const [probe, setProbe] = useState("");
  const [tunStack, setTunStack] = useState("mixed");
  /** IPv6 address on the TUN interface. Off by default — most nodes have no
   * v6 egress and a dual-stack tun makes Chrome prefer AAAA/v6, black-holing
   * every connection. */
  const [tunIpv6, setTunIpv6] = useState(false);
  /** Reject sniffed QUIC (UDP/443) so browsers fall back to TCP. */
  const [blockQuic, setBlockQuic] = useState(false);
  /** Bypass localhost and LAN segments with built-in direct rules. */
  const [bypassLan, setBypassLan] = useState(true);
  /** Extra inbound drafts — applied on card save (needs core restart). */
  const [extra, setExtra] = useState<ExtraInbound[]>([]);
  // Extra-inbound editor modal (add / edit share one form).
  const [inboundOpen, setInboundOpen] = useState(false);
  const [inboundEditId, setInboundEditId] = useState<string | null>(null);
  const [inboundKind, setInboundKind] = useState<"mixed" | "http">("mixed");
  const [inboundPort, setInboundPort] = useState("");
  const [inboundLan, setInboundLan] = useState(false);
  const [inboundError, setInboundError] = useState<string | null>(null);
  const [menuInboundId, setMenuInboundId] = useState<string | null>(null);
  /** Copy-feedback flag for the read-only Clash API secret field. */
  const [secretCopied, setSecretCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** Detection-only network diagnostics (e.g. system DNS bypassing TUN).
   * Re-checked whenever TUN transitions off → on; never auto-applied. */
  const [netDiagnostics, setNetDiagnostics] = useState<DiagnosticIssue[]>([]);

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
          id: "ports" as const,
          label: t("settings.tabPorts"),
          hint: t("settings.hintPorts"),
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
        setAllowLan(!!s.allow_lan);
        setApi(String(s.api_port));
        setProbe(s.probe_url);
        setTunStack(s.tun_stack || "mixed");
        setTunIpv6(!!s.tun_ipv6_enabled);
        setBlockQuic(!!s.block_quic);
        setBypassLan(s.bypass_lan !== false);
        setExtra(s.extra_inbounds ?? []);
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

  // Close the inbound-row ⋮ menu on outside pointer-down / Escape.
  useEffect(() => {
    if (!menuInboundId) return;
    function onDocPointerDown(e: PointerEvent) {
      const t = e.target as HTMLElement | null;
      if (t?.closest?.("[data-inbound-menu]")) return;
      setMenuInboundId(null);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setMenuInboundId(null);
    }
    document.addEventListener("pointerdown", onDocPointerDown, true);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDocPointerDown, true);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuInboundId]);

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

  /** Latest auto-apply fn (called from the debounced effect and re-queued
   * from its own finally when the user edited mid-flight). */
  const autoApplyRef = useRef<() => Promise<void>>(async () => {});
  const applyingRef = useRef(false);
  /** Previous tun_enabled value, to detect the off → on transition. */
  const prevTunEnabledRef = useRef<boolean | undefined>(undefined);

  // Re-run detection-only network diagnostics whenever TUN turns on. Never
  // fires on every settings refresh — only on the actual off → on edge —
  // since the check involves a couple of shell-outs on macOS.
  useEffect(() => {
    const wasOn = prevTunEnabledRef.current;
    const isOn = !!settings?.tun_enabled;
    prevTunEnabledRef.current = isOn;
    if (wasOn === undefined || wasOn === isOn || !isOn) {
      if (!isOn) setNetDiagnostics([]);
      return;
    }
    diagnoseNetwork()
      .then((r) => setNetDiagnostics(r.issues))
      .catch(() => setNetDiagnostics([]));
  }, [settings?.tun_enabled]);

  /** Auto-commit the ports tab: save every draft (ports / LAN / probe /
   * stack / listeners) and restart the core when it is running. Drafts that
   * are still invalid (mid-typing) are skipped until they become valid. */
  const autoApplyNetwork = useCallback(async () => {
    if (applyingRef.current || !settings) return;
    const dirty =
      String(settings.mixed_port) !== mixed.trim() ||
      !!settings.allow_lan !== allowLan ||
      String(settings.api_port) !== api.trim() ||
      (settings.probe_url ?? "") !== probe ||
      (settings.tun_stack || "mixed") !== tunStack ||
      !!settings.tun_ipv6_enabled !== tunIpv6 ||
      !!settings.block_quic !== blockQuic ||
      (settings.bypass_lan !== false) !== bypassLan ||
      !sameInbounds(settings.extra_inbounds ?? [], extra);
    if (!dirty) return;
    // Invalid drafts (mid-typing or left behind): surface why we can't apply
    // yet; the banner clears on the next successful auto-commit.
    const mixedPort = Number(mixed);
    const apiPort = Number(api);
    if (!Number.isFinite(mixedPort) || mixedPort < 1 || mixedPort > 65535) {
      setError(t("settings.invalidMixed"));
      return;
    }
    if (!Number.isFinite(apiPort) || apiPort < 1 || apiPort > 65535) {
      setError(t("settings.invalidApi"));
      return;
    }
    const seen = new Set<number>([mixedPort, apiPort]);
    for (const row of extra) {
      if (seen.has(row.port)) {
        setError(t("settings.dupPort", { n: row.port }));
        return;
      }
      seen.add(row.port);
    }
    applyingRef.current = true;
    setBusy(true);
    setError(null);
    try {
      const s = await updateSettings({
        mixedPort,
        allowLan,
        apiPort,
        extraInbounds: extra,
        probeUrl: probe.trim() || null,
        tunStack: tunStack.trim() || "mixed",
        tunIpv6Enabled: tunIpv6,
        blockQuic,
        bypassLan,
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
      applyingRef.current = false;
      setBusy(false);
      // Pick up edits made while we were applying.
      void autoApplyRef.current();
    }
  }, [allowLan, api, blockQuic, bypassLan, extra, mixed, probe, settings, t, tunIpv6, tunStack]);

  autoApplyRef.current = autoApplyNetwork;

  // Debounce so typing a port number doesn't restart the core per keystroke;
  // toggles / selects / modal saves settle within the same short window.
  useEffect(() => {
    if (!settings) return;
    const timer = setTimeout(() => void autoApplyRef.current(), 600);
    return () => clearTimeout(timer);
    // Fire on any draft change; autoApplyNetwork itself decides if there is
    // anything valid and dirty to commit.
  }, [settings, mixed, allowLan, api, probe, tunStack, tunIpv6, blockQuic, bypassLan, extra]);

  // —— Extra inbound listeners (draft rows + modal editor) ——

  function openAddInbound() {
    setInboundEditId(null);
    setInboundKind("mixed");
    setInboundPort("");
    setInboundLan(false);
    setInboundError(null);
    setInboundOpen(true);
  }

  function openEditInbound(row: ExtraInbound) {
    setInboundEditId(row.id);
    setInboundKind(row.kind);
    setInboundPort(String(row.port));
    setInboundLan(!!row.allow_lan);
    setInboundError(null);
    setInboundOpen(true);
  }

  /** Validate in the modal, then commit to the list (auto-applied + core
   * restart via the debounced effect). */
  function saveInbound() {
    const port = Number(inboundPort);
    if (!Number.isFinite(port) || port < 1 || port > 65535) {
      setInboundError(t("settings.invalidExtraPort"));
      return;
    }
    const others = extra.filter((r) => r.id !== inboundEditId);
    const taken = new Set<number>([
      ...others.map((r) => r.port),
      settings?.mixed_port ?? 0,
      settings?.api_port ?? 0,
    ]);
    if (taken.has(port)) {
      setInboundError(t("settings.dupPort", { n: port }));
      return;
    }
    const entry: ExtraInbound = {
      id: inboundEditId ?? `in-${Math.random().toString(36).slice(2, 10)}`,
      kind: inboundKind,
      port,
      allow_lan: inboundLan,
    };
    setExtra((prev) =>
      inboundEditId == null
        ? [...prev, entry]
        : prev.map((r) => (r.id === inboundEditId ? entry : r)),
    );
    setInboundOpen(false);
  }

  function removeInbound(id: string) {
    setExtra((prev) => prev.filter((r) => r.id !== id));
  }

  async function onCopySecret() {
    const secret = settings?.clash_api_secret;
    if (!secret) return;
    try {
      await navigator.clipboard.writeText(secret);
      setSecretCopied(true);
      window.setTimeout(() => setSecretCopied(false), 1500);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }

  /** User-triggered secret rotation; backend restarts a running core so the
   * new secret is live immediately. */
  async function onRegenerateSecret() {
    setError(null);
    setBusy(true);
    try {
      const s = await regenerateApiSecret();
      setSettings(s);
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

  // The sponsor easter egg renders on the app tab only; leaving the tab
  // unmounts it — reset the open state so coming back doesn't resurrect
  // a stale panel.
  useEffect(() => {
    if (visibleTab !== "app") setSponsorOpen(false);
  }, [visibleTab]);

  const needsSettings =
    visibleTab === "app" || visibleTab === "ports" || visibleTab === "core";
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

      {/* Sponsor QR — portaled to <body> so a transformed ancestor (the
       * page-enter animation wrapper) cannot become the containing block
       * for these position:fixed nodes. App tab only. */}
      {visibleTab === "app" &&
        createPortal(
          <>
            <button
              type="button"
              className="sponsor-link"
              onClick={(e) => {
                e.stopPropagation();
                setSponsorOpen((v) => !v);
              }}
            >
              {t("settings.sponsor")}
            </button>
            {sponsorOpen && (
              <div
                className="sponsor-panel"
                role="dialog"
                aria-label={t("settings.sponsor")}
                onClick={(e) => e.stopPropagation()}
              >
                <DecryptReveal radius={140} dismissOnLeave>
                  <img
                    className="sponsor-qr"
                    src={buymecoffeeUrl}
                    alt={t("settings.sponsorScan")}
                    draggable={false}
                  />
                </DecryptReveal>
                <div className="sponsor-caption muted">
                  {t("settings.sponsorScan")}
                </div>
              </div>
            )}
          </>,
          document.body,
        )}

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
        className={`page-enter${
          visibleTab === "app"
            ? " settings-app-page"
            : visibleTab === "ports"
              ? " settings-ports-page"
              : ""
        }${
          visibleTab === "rules" || visibleTab === "dns"
            ? " settings-scroll-embed"
            : ""
        }`}
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

      {visibleTab === "ports" && settings && (
        <section className="settings-panel" aria-label="Ports">
          {/* Note only — every change auto-commits below (and restarts a
            running core); there is no save button on this tab. */}
          <div className="settings-network-card-head settings-ports-toolbar">
            <div>
              <strong>{t("settings.networkOptions")}</strong>
              <div className="muted">{t("settings.networkSaveNote")}</div>
            </div>
          </div>
          <div className="settings-ports-columns">
            <div className="card settings-form settings-form-grid">
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
              <div className="field field-span-2">
                <span>{t("settings.apiSecret")}</span>
                <div className="api-secret-row">
                  <input
                    readOnly
                    autoCapitalize="off"
                    autoCorrect="off"
                    spellCheck={false}
                    className="mono api-secret-input"
                    value={settings?.clash_api_secret ?? ""}
                    placeholder={t("settings.apiSecretNone")}
                  />
                  <GlassButton
                    icon={secretCopied ? "✓" : "⧉"}
                    disabled={!settings?.clash_api_secret}
                    onClick={() => void onCopySecret()}
                    title={t("common.copy")}
                  >
                    {secretCopied ? t("common.copied") : t("common.copy")}
                  </GlassButton>
                  <GlassButton
                    icon="↻"
                    disabled={busy || customRuntime}
                    onClick={() => void onRegenerateSecret()}
                    title={t("settings.regenerateSecret")}
                  >
                    {t("settings.regenerateSecret")}
                  </GlassButton>
                </div>
                <span className="field-hint muted">
                  {t("settings.apiSecretHint")}
                </span>
              </div>
              <div className="via-proxy-row field-span-2">
                <div>
                  <div className="sys-proxy-title">{t("settings.allowLan")}</div>
                  <div className="sys-proxy-desc">
                    {t("settings.allowLanDesc")}
                  </div>
                </div>
                <GlassSwitchControl
                  checked={allowLan}
                  title={t("settings.allowLan")}
                  disabled={busy || (settings?.runtime_source ?? "").startsWith("singbox:")}
                  onChange={setAllowLan}
                />
              </div>
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
              <div className="via-proxy-row field-span-2">
                <div>
                  <div className="sys-proxy-title">{t("settings.tunIpv6")}</div>
                  <div className="sys-proxy-desc">{t("settings.tunIpv6Desc")}</div>
                </div>
                <GlassSwitchControl
                  checked={tunIpv6}
                  title={t("settings.tunIpv6")}
                  disabled={busy}
                  onChange={setTunIpv6}
                />
              </div>
              <div className="via-proxy-row field-span-2">
                <div>
                  <div className="sys-proxy-title">{t("settings.blockQuic")}</div>
                  <div className="sys-proxy-desc">{t("settings.blockQuicDesc")}</div>
                </div>
                <GlassSwitchControl
                  checked={blockQuic}
                  title={t("settings.blockQuic")}
                  disabled={busy}
                  onChange={setBlockQuic}
                />
              </div>
              <div className="via-proxy-row field-span-2">
                <div>
                  <div className="sys-proxy-title">{t("settings.bypassLan")}</div>
                  <div className="sys-proxy-desc">{t("settings.bypassLanDesc")}</div>
                </div>
                <GlassSwitchControl
                  checked={bypassLan}
                  title={t("settings.bypassLan")}
                  disabled={busy}
                  onChange={setBypassLan}
                />
              </div>
              {netDiagnostics.length > 0 && (
                <div className="field-span-2 diagnostic-banner-list">
                  {netDiagnostics.map((d) => (
                    <div className="diagnostic-banner" key={d.id}>
                      <div className="diagnostic-banner-issue">{d.issue}</div>
                      <div className="diagnostic-banner-suggestion">
                        {d.suggestion}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
            <div className="card settings-form settings-inbounds-card">
              <div className="settings-network-card-head">
                <div>
                  <strong>{t("settings.extraInbounds")}</strong>
                  <div className="muted">{t("settings.extraInboundsDesc")}</div>
                </div>
                <GlassButton
                  icon="+"
                  disabled={busy || customRuntime || extra.length >= 10}
                  onClick={openAddInbound}
                >
                  {t("settings.addInboundPort")}
                </GlassButton>
              </div>
              <div className="table-wrap inbound-table-wrap">
                <table className="inbound-table">
                  <colgroup>
                    <col style={{ width: 100 }} />
                    <col />
                    <col style={{ width: 60 }} />
                  </colgroup>
                  <thead>
                    <tr>
                      <th>{t("settings.inboundType")}</th>
                      <th>{t("settings.inboundAddr")}</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    {extra.length === 0 ? (
                      <tr>
                        <td colSpan={3} className="muted inbound-empty">
                          {t("settings.extraInboundsEmpty")}
                        </td>
                      </tr>
                    ) : (
                      extra.map((row) => (
                        <tr key={row.id}>
                          <td>
                            <code>{row.kind}</code>
                          </td>
                          <td className="mono">
                            {row.allow_lan ? "0.0.0.0" : "127.0.0.1"}:{row.port}
                          </td>
                          <td>
                            <div className="rule-menu" data-inbound-menu>
                              <button
                                type="button"
                                className="rule-menu-trigger"
                                aria-label={t("common.edit")}
                                aria-haspopup="menu"
                                aria-expanded={menuInboundId === row.id}
                                disabled={busy}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  setMenuInboundId((id) =>
                                    id === row.id ? null : row.id,
                                  );
                                }}
                              >
                                ⋮
                              </button>
                              {menuInboundId === row.id && (
                                <div className="rule-menu-pop" role="menu">
                                  <button
                                    type="button"
                                    role="menuitem"
                                    className="rule-menu-item"
                                    onClick={() => {
                                      setMenuInboundId(null);
                                      openEditInbound(row);
                                    }}
                                  >
                                    {t("common.edit")}
                                  </button>
                                  <button
                                    type="button"
                                    role="menuitem"
                                    className="rule-menu-item danger"
                                    onClick={() => {
                                      setMenuInboundId(null);
                                      removeInbound(row.id);
                                    }}
                                  >
                                    {t("common.delete")}
                                  </button>
                                </div>
                              )}
                            </div>
                          </td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </div>
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

      {inboundOpen && (
        <div className="modal-backdrop">
          <div className="modal">
            <header className="modal-header">
              <h2>
                {inboundEditId
                  ? t("settings.editInboundTitle")
                  : t("settings.addInboundPort")}
              </h2>
              <button
                type="button"
                className="icon-btn"
                onClick={() => setInboundOpen(false)}
                disabled={busy}
                aria-label={t("common.cancel")}
              >
                ×
              </button>
            </header>
            <form
              className="modal-body"
              onSubmit={(e) => {
                e.preventDefault();
                void saveInbound();
              }}
            >
              <div className="field">
                <span>{t("settings.inboundType")}</span>
                <GlassSeg
                  value={inboundKind}
                  ariaLabel={t("settings.inboundType")}
                  disabled={busy}
                  onChange={(v) => setInboundKind(v as "mixed" | "http")}
                  options={[
                    { value: "mixed", label: "mixed" },
                    { value: "http", label: "http" },
                  ]}
                />
              </div>
              <label className="field">
                <span>{t("settings.portLabel")}</span>
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  className="mono"
                  value={inboundPort}
                  onChange={(e) => setInboundPort(e.target.value)}
                  placeholder="8080"
                  disabled={busy}
                  autoFocus
                />
              </label>
              <div className="via-proxy-row">
                <div>
                  <div className="sys-proxy-title">{t("settings.allowLan")}</div>
                  <div className="sys-proxy-desc">{t("settings.allowLanDesc")}</div>
                </div>
                <GlassSwitchControl
                  checked={inboundLan}
                  title={t("settings.allowLan")}
                  disabled={busy}
                  onChange={setInboundLan}
                />
              </div>
              {inboundError && <div className="form-error">{inboundError}</div>}
              <footer className="modal-footer">
                <GlassButton onClick={() => setInboundOpen(false)} disabled={busy}>
                  {t("common.cancel")}
                </GlassButton>
                <GlassButton type="submit" variant="primary" disabled={busy}>
                  {busy ? t("common.saving") : t("common.save")}
                </GlassButton>
              </footer>
            </form>
          </div>
        </div>
      )}
      </div>
    </div>
  );
}

/** Order-sensitive equality for the extra-inbound draft list. */
function sameInbounds(a: ExtraInbound[], b: ExtraInbound[]) {
  if (a.length !== b.length) return false;
  return a.every((x, i) => {
    const y = b[i];
    return (
      x.id === y.id &&
      x.kind === y.kind &&
      x.port === y.port &&
      !!x.allow_lan === !!y.allow_lan
    );
  });
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
