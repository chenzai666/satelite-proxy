import { useEffect, useState } from "react";
import { TopNav } from "./components/TopNav";
import { ImportIntentProvider, useImportIntent } from "./ImportIntentContext";
import { LocaleProvider } from "./i18n";
import { ThemeProvider } from "./theme";
import { ConfigPage } from "./pages/ConfigPage";
import { DashboardPage } from "./pages/DashboardPage";
import { LogsPage } from "./pages/LogsPage";
import { NodesPage } from "./pages/NodesPage";
import { TrafficPage } from "./pages/TrafficPage";
import { SettingsPage } from "./pages/SettingsPage";
import type { NavKey } from "./types";
import { UiModeProvider, useUiMode } from "./ui/UiModeContext";
import { SimpleShell } from "./ui/simple";
import "./App.css";

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
        {nav === "dashboard" && (
          <DashboardPage
            onGoProfiles={() => setNav("config")}
            onGoNodes={() => setNav("nodes")}
            onGoTraffic={() => setNav("traffic")}
            onGoSettings={() => setNav("settings")}
          />
        )}
        {nav === "config" && <ConfigPage />}
        {nav === "nodes" && <NodesPage />}
        {nav === "traffic" && <TrafficPage />}
        {nav === "logs" && <LogsPage />}
        {nav === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}

function AppShell() {
  const { mode, layoutReady } = useUiMode();
  // Hold a blank shell until window size matches mode — no pro→simple flash on wake.
  if (!layoutReady) {
    return <div className="app-shell ui-boot-shell" aria-busy="true" />;
  }
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
