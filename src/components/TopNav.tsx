import { useCallback, useEffect, useState } from "react";
import { getCoreInfo, getProxyStatus } from "../api";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import type { NavKey } from "../types";

type NavItem = { key: NavKey; label: string };

/** Compact capsule order — style3 horizontal nav (labels stay English). */
const ITEMS: NavItem[] = [
  { key: "dashboard", label: "Overview" },
  { key: "nodes", label: "Nodes" },
  { key: "config", label: "Profiles" },
  { key: "traffic", label: "Traffic" },
  { key: "logs", label: "Logs" },
  { key: "settings", label: "Settings" },
];

interface Props {
  active: NavKey;
  onChange: (key: NavKey) => void;
}

export function TopNav({ active, onChange }: Props) {
  const [coreVersion, setCoreVersion] = useState("—");
  const [running, setRunning] = useState(false);
  const [coreState, setCoreState] = useState("stopped");

  const tick = useCallback(async () => {
    try {
      const [core, status] = await Promise.all([
        getCoreInfo().catch(() => null),
        getProxyStatus().catch(() => null),
      ]);
      if (core?.installed && core.version) {
        setCoreVersion(core.version.replace(/^v/, ""));
      } else if (core?.bundled_version) {
        setCoreVersion(String(core.bundled_version).replace(/^v/, ""));
      } else {
        setCoreVersion(core?.installed ? "ok" : "—");
      }
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
    void tick();
  }, 3000);

  const stateLabel = running
    ? "RUN"
    : coreState === "starting"
      ? "…"
      : coreState === "error"
        ? "ERR"
        : "OFF";
  const dotClass =
    running || coreState === "running"
      ? "on"
      : coreState === "starting" || coreState === "stopping"
        ? "busy"
        : "off";

  return (
    <header className="topnav-wrap">
      <div className="topnav" role="navigation" aria-label="Main">
        <div className="topnav-brand" title="Satelite">
          <span className="topnav-mark" aria-hidden>
            ◈
          </span>
          <span className="topnav-brand-text">SATELITE</span>
        </div>
        <div className="topnav-divider" aria-hidden />
        <nav className="topnav-items">
          {ITEMS.map((item) => (
            <button
              key={item.key}
              type="button"
              className={`topnav-item ${active === item.key ? "active" : ""}`}
              onClick={() => onChange(item.key)}
            >
              {item.label}
            </button>
          ))}
        </nav>
        <div className="topnav-status" title={`sing-box v${coreVersion}`}>
          <span className={`status-dot ${dotClass}`} />
          <span className="topnav-status-text">{stateLabel}</span>
          <span className="topnav-status-ver">v{coreVersion}</span>
        </div>
      </div>
    </header>
  );
}
