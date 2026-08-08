import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { getCoreInfo, getProxyStatus } from "../api";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import { useTheme } from "../theme";
import type { NavKey } from "../types";
import { UiModeMenu } from "../ui/UiModeMenu";

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
  const { theme, setTheme } = useTheme();
  const [coreVersion, setCoreVersion] = useState("—");
  const [running, setRunning] = useState(false);
  const [coreState, setCoreState] = useState("stopped");

  // Sliding highlight indicator: measure the active button's position/size.
  const itemRefs = useRef<Record<string, HTMLButtonElement>>({});
  const [indicatorStyle, setIndicatorStyle] = useState<CSSProperties>({
    opacity: 0,
  });
  useLayoutEffect(() => {
    const el = itemRefs.current[active];
    if (!el) return;
    setIndicatorStyle({
      opacity: 1,
      transform: `translateX(${el.offsetLeft}px)`,
      width: `${el.offsetWidth}px`,
    });
  }, [active]);

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
          {/* Sliding highlight: positioned over the active button via layout
              effect measurements below. Width is fixed by the ref callback so
              the pill travels smoothly between unequal-width items. */}
          <span
            className="topnav-indicator"
            aria-hidden="true"
            style={indicatorStyle}
          />
          {ITEMS.map((item) => (
            <button
              key={item.key}
              type="button"
              ref={(el) => {
                if (el) itemRefs.current[item.key] = el;
              }}
              className={`topnav-item ${active === item.key ? "active" : ""}`}
              onClick={() => onChange(item.key)}
            >
              {item.label}
            </button>
          ))}
        </nav>
        <div className="topnav-tools">
          <div
            className="topnav-theme-switch"
            role="group"
            aria-label="外观"
          >
            <button
              type="button"
              className={`topnav-theme-btn ${theme === "day" ? "active" : ""}`}
              aria-label="亮色模式"
              aria-pressed={theme === "day"}
              title="Day"
              onClick={() => void setTheme("day")}
            >
              ☼
            </button>
            <button
              type="button"
              className={`topnav-theme-btn ${theme === "aerospace" ? "active" : ""}`}
              aria-label="暗色模式"
              aria-pressed={theme === "aerospace"}
              title="Mission"
              onClick={() => void setTheme("aerospace")}
            >
              ◐
            </button>
          </div>
          <div className="topnav-status" title={`sing-box v${coreVersion}`}>
            <span className={`status-dot ${dotClass}`} />
            <span className="topnav-status-text">{stateLabel}</span>
            <span className="topnav-status-ver">v{coreVersion}</span>
          </div>
          <UiModeMenu />
        </div>
      </div>
    </header>
  );
}
