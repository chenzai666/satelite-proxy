import { lazy, Suspense, useEffect, useState } from "react";
import { useImportIntent } from "../../ImportIntentContext";
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

const TABS: { key: SimpleNavKey; label: string }[] = [
  { key: "connect", label: "连接" },
  { key: "servers", label: "节点" },
  { key: "traffic", label: "流量" },
  { key: "settings", label: "设置" },
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
  const [nav, setNav] = useState<SimpleNavKey>("connect");
  const { token, prefill } = useImportIntent();

  // One-click subscribe → open 节点 page (add subscription modal).
  useEffect(() => {
    if (token && prefill) setNav("servers");
  }, [token, prefill]);

  return (
    <div className="app-shell simple-shell">
      <header className="topnav-wrap simple-topnav-wrap">
        <div
          className="topnav simple-topnav"
          role="navigation"
          aria-label="Simple"
        >
          <div className="topnav-brand simple-brand" title="Satelite">
            <span className="topnav-mark" aria-hidden>
              ◇
            </span>
          </div>
          <nav className="topnav-items simple-topnav-items">
            {TABS.map((item) => (
              <button
                key={item.key}
                type="button"
                className={`topnav-item ${nav === item.key ? "active" : ""}`}
                onClick={() => setNav(item.key)}
              >
                {item.label}
              </button>
            ))}
          </nav>
          <UiModeMenu />
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
