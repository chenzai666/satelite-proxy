import { lazy, Suspense, useCallback, useEffect, useState } from "react";
import { getProxyStatus } from "../../api";
import { ThemeSwitch } from "../../components/ThemeSwitch";
import { useCoreBusy } from "../../coreBusy";
import { useVisibleInterval } from "../../hooks/useVisibleInterval";
import { useImportIntent } from "../../ImportIntentContext";
import { useI18n } from "../../i18n";
import type { MessageKey } from "../../i18n";
import { UiModeMenu } from "../UiModeMenu";
import { SimpleConnectPage } from "./SimpleConnectPage";

export type SimpleNavKey = "connect" | "servers" | "traffic" | "settings";

const SimpleServersPage = lazy(() =>
  import("./SimpleServersPage").then((m) => ({ default: m.SimpleServersPage })),
);
const SimpleTrafficPage = lazy(() =>
  import("./SimpleTrafficPage").then((m) => ({ default: m.SimpleTrafficPage })),
);
const SimpleSettingsPage = lazy(() =>
  import("./SimpleSettingsPage").then((m) => ({
    default: m.SimpleSettingsPage,
  })),
);

const TABS: { key: SimpleNavKey; labelKey: MessageKey }[] = [
  { key: "connect", labelKey: "nav.connect" },
  { key: "servers", labelKey: "nodes.title" },
  { key: "traffic", labelKey: "traffic.title" },
  { key: "settings", labelKey: "settings.title" },
];

function SimplePageFallback() {
  return (
    <div className="simple-page" aria-busy="true">
      <div className="skel skel-block" />
      <div className="skel skel-line skel-w-60" />
      <div className="skel skel-line skel-w-40" />
    </div>
  );
}

export function SimpleShell() {
  const { t } = useI18n();
  const coreBusy = useCoreBusy();
  const [nav, setNav] = useState<SimpleNavKey>("connect");
  const [running, setRunning] = useState(false);
  const [coreState, setCoreState] = useState("stopped");
  const { token, prefill } = useImportIntent();

  // One-click subscribe → open 节点 page (add subscription modal).
  useEffect(() => {
    if (token && prefill) setNav("servers");
  }, [token, prefill]);

  const tick = useCallback(async () => {
    try {
      const status = await getProxyStatus().catch(() => null);
      setRunning(status?.running ?? false);
      setCoreState(status?.core_state ?? "stopped");
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    void tick();
  }, [tick]);

  useVisibleInterval(() => {
    if (coreBusy) return;
    void tick();
  }, 3000);

  useEffect(() => {
    if (!coreBusy) void tick();
  }, [coreBusy, tick]);

  const transitioning =
    coreBusy ||
    coreState === "starting" ||
    coreState === "stopping";
  const stateLabel = transitioning
    ? "…"
    : running
      ? "RUN"
      : coreState === "error"
        ? "ERR"
        : "OFF";
  const dotClass = transitioning
    ? "busy"
    : running || coreState === "running"
      ? "on"
      : "off";

  return (
    <div className="app-shell simple-shell">
      <header className="topnav-wrap simple-topnav-wrap">
        <div
          className="topnav simple-topnav"
          role="navigation"
          aria-label="Simple"
        >
          <nav className="topnav-items simple-topnav-items">
            {TABS.map((item) => (
              <button
                key={item.key}
                type="button"
                className={`topnav-item ${nav === item.key ? "active" : ""}`}
                onClick={() => setNav(item.key)}
              >
                {t(item.labelKey)}
              </button>
            ))}
          </nav>
          <div className="topnav-tools simple-topnav-tools">
            <ThemeSwitch />
            <div
              className="topnav-status"
              title={transitioning ? "内核切换中" : stateLabel}
              aria-busy={transitioning}
            >
              <span className={`status-dot ${dotClass}`} />
              <span className="topnav-status-text">{stateLabel}</span>
            </div>
            <UiModeMenu />
          </div>
        </div>
      </header>
      <main className="main simple-main">
        {nav === "connect" && (
          <SimpleConnectPage onGoServers={() => setNav("servers")} />
        )}
        {nav !== "connect" && (
          <Suspense fallback={<SimplePageFallback />}>
            {nav === "servers" && <SimpleServersPage />}
            {nav === "traffic" && <SimpleTrafficPage />}
            {nav === "settings" && <SimpleSettingsPage />}
          </Suspense>
        )}
      </main>
    </div>
  );
}
