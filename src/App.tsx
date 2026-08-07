import { lazy, Suspense, useEffect, useState } from "react";
import { TopNav } from "./components/TopNav";
import { ImportIntentProvider, useImportIntent } from "./ImportIntentContext";
import { LocaleProvider } from "./i18n";
import { ThemeProvider } from "./theme";
import { DashboardPage } from "./pages/DashboardPage";
import type { NavKey } from "./types";
import { UiModeProvider, useUiMode } from "./ui/UiModeContext";
import { SimpleShell } from "./ui/simple";
import "./App.css";

// Secondary pages: code-split so low-memory WebView recreate only parses home first.
const ConfigPage = lazy(() =>
  import("./pages/ConfigPage").then((m) => ({ default: m.ConfigPage })),
);
const NodesPage = lazy(() =>
  import("./pages/NodesPage").then((m) => ({ default: m.NodesPage })),
);
const TrafficPage = lazy(() =>
  import("./pages/TrafficPage").then((m) => ({ default: m.TrafficPage })),
);
const LogsPage = lazy(() =>
  import("./pages/LogsPage").then((m) => ({ default: m.LogsPage })),
);
const SettingsPage = lazy(() =>
  import("./pages/SettingsPage").then((m) => ({ default: m.SettingsPage })),
);

function PageFallback() {
  return (
    <div className="page page-fallback" aria-busy="true">
      <div className="skel skel-line skel-w-40" />
      <div className="skel skel-block" />
      <div className="skel skel-line skel-w-60" />
      <div className="skel skel-line skel-w-50" />
    </div>
  );
}

function ProShell() {
  const [nav, setNav] = useState<NavKey>("dashboard");
  const { token, prefill } = useImportIntent();

  // One-click subscribe → jump to profiles so ConfigPage can open the add form.
  useEffect(() => {
    if (token && prefill) setNav("config");
  }, [token, prefill]);

  return (
    <div className="app-shell">
      <TopNav active={nav} onChange={setNav} />
      <main className="main">
        {/* key={nav} forces a remount on page switch → triggers the CSS
            page-enter fade/slide animation below. */}
        <div className="page-enter" key={nav}>
          {nav === "dashboard" && (
            <DashboardPage
              onGoProfiles={() => setNav("config")}
              onGoNodes={() => setNav("nodes")}
              onGoTraffic={() => setNav("traffic")}
              onGoSettings={() => setNav("settings")}
            />
          )}
          {nav !== "dashboard" && (
            <Suspense fallback={<PageFallback />}>
              {nav === "config" && <ConfigPage />}
              {nav === "nodes" && <NodesPage />}
              {nav === "traffic" && <TrafficPage />}
              {nav === "logs" && <LogsPage />}
              {nav === "settings" && <SettingsPage />}
            </Suspense>
          )}
        </div>
      </main>
    </div>
  );
}

function AppShell() {
  const { mode } = useUiMode();
  // Paint immediately from localStorage mode (Rust already sized window on recreate).
  return mode === "simple" ? <SimpleShell /> : <ProShell />;
}

function App() {
  return (
    <ThemeProvider>
      <LocaleProvider>
        <UiModeProvider>
          <ImportIntentProvider>
            <AppShell />
          </ImportIntentProvider>
        </UiModeProvider>
      </LocaleProvider>
    </ThemeProvider>
  );
}

export default App;
