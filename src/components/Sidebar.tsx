import { useEffect, useState } from "react";
import { getCoreInfo, getProxyStatus } from "../api";
import type { NavKey } from "../types";

type NavItem = { key: NavKey; label: string; icon: string; enabled: boolean };

const GROUPS: { id: string; label: string | null; items: NavItem[] }[] = [
  {
    id: "main",
    label: null,
    items: [{ key: "dashboard", label: "Dashboard", icon: "◉", enabled: true }],
  },
  {
    id: "network",
    label: "Network",
    items: [
      { key: "nodes", label: "Nodes", icon: "◎", enabled: true },
      { key: "traffic", label: "Traffic", icon: "⌁", enabled: true },
    ],
  },
  {
    id: "system",
    label: "System",
    items: [
      { key: "config", label: "Profiles", icon: "▣", enabled: true },
      { key: "logs", label: "Logs", icon: "▤", enabled: true },
      { key: "settings", label: "Settings", icon: "⚙", enabled: true },
    ],
  },
];

interface Props {
  active: NavKey;
  onChange: (key: NavKey) => void;
}

export function Sidebar({ active, onChange }: Props) {
  const [coreVersion, setCoreVersion] = useState<string>("—");
  const [running, setRunning] = useState(false);
  const [coreState, setCoreState] = useState("stopped");

  useEffect(() => {
    let cancelled = false;

    async function tick() {
      try {
        const [core, status] = await Promise.all([
          getCoreInfo().catch(() => null),
          getProxyStatus().catch(() => null),
        ]);
        if (cancelled) return;
        if (core?.installed && core.version) {
          setCoreVersion(core.version.replace(/^v/, ""));
        } else if (core?.bundled_version) {
          setCoreVersion(String(core.bundled_version).replace(/^v/, ""));
        } else {
          setCoreVersion(core?.installed ? "ok" : "missing");
        }
        setRunning(status?.running ?? false);
        setCoreState(status?.core_state ?? "stopped");
      } catch {
        /* ignore */
      }
    }

    void tick();
    const t = window.setInterval(() => void tick(), 2500);
    return () => {
      cancelled = true;
      window.clearInterval(t);
    };
  }, []);

  const stateLabel = running
    ? "Running"
    : coreState === "starting"
      ? "Starting"
      : coreState === "stopping"
        ? "Stopping"
        : coreState === "error"
          ? "Error"
          : "Stopped";
  const dotClass =
    running || coreState === "running"
      ? "on"
      : coreState === "starting" || coreState === "stopping"
        ? "busy"
        : "off";

  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark">◈</div>
        <div>
          <div className="brand-title">SATELITE</div>
          <div className="brand-sub">sing-box engine</div>
        </div>
      </div>

      <nav className="nav">
        {GROUPS.map((group) => (
          <div key={group.id} className="nav-group">
            {group.label && (
              <div className="nav-group-label">{group.label}</div>
            )}
            {group.items.map((item) => (
              <button
                key={item.key}
                type="button"
                className={`nav-item ${active === item.key ? "active" : ""} ${
                  item.enabled ? "" : "disabled"
                }`}
                disabled={!item.enabled}
                onClick={() => item.enabled && onChange(item.key)}
                title={item.enabled ? undefined : "Coming soon"}
              >
                <span className="nav-icon" aria-hidden>
                  {item.icon}
                </span>
                {item.label}
              </button>
            ))}
          </div>
        ))}
      </nav>

      <div className="sidebar-footer">
        <div className="sidebar-footer-title">sing-box</div>
        <div>v{coreVersion}</div>
        <div className="sidebar-footer-row">
          <span className={`status-dot ${dotClass}`} />
          <span>{stateLabel}</span>
        </div>
      </div>
    </aside>
  );
}
