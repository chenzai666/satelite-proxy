import { useState } from "react";
import { TopNav } from "./components/TopNav";
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
  const { mode } = useUiMode();
  return mode === "simple" ? <SimpleShell /> : <ProShell />;
}

function App() {
  return (
    <ThemeProvider>
      <LocaleProvider>
        <UiModeProvider>
          <AppShell />
        </UiModeProvider>
      </LocaleProvider>
    </ThemeProvider>
  );
}

export default App;
