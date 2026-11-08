import { useEffect, useId, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/messages";
import type { NavKey } from "../types";
import { UiModeMenu } from "../ui/UiModeMenu";
import { ThemeSwitch } from "./ThemeSwitch";
import "./SideNav.css";

const COMPACT_QUERY = "(max-width: 1100px)";
const ITEMS: { key: NavKey; labelKey: MessageKey; path: string }[] = [
  { key: "dashboard", labelKey: "dashboard.title", path: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z" },
  { key: "nodes", labelKey: "nodes.title", path: "M8 5h8 M6 8v8 M18 8v8 M8 19h8 M3 3h5v5H3z M16 3h5v5h-5z M3 16h5v5H3z M16 16h5v5h-5z" },
  { key: "config", labelKey: "config.title", path: "M3 6h7l2 2h9v12H3z M3 6V4h7l2 2h7v2" },
  { key: "traffic", labelKey: "traffic.title", path: "M3 20h18 M6 16v-5 M12 16V4 M18 16V8" },
  { key: "logs", labelKey: "logs.title", path: "M5 3h14v18H5z M8 7h8 M8 12h8 M8 17h5" },
  { key: "settings", labelKey: "settings.title", path: "M4 6h4 M12 6h8 M4 12h10 M18 12h2 M4 18h2 M10 18h10 M8 3v6 M14 9v6 M6 15v6" },
];

interface Props {
  active: NavKey;
  onChange: (key: NavKey) => void;
}

export function SideNav({ active, onChange }: Props) {
  const { t } = useI18n();
  const navId = useId();
  const [collapsed, setCollapsed] = useState(() => window.matchMedia(COMPACT_QUERY).matches);
  const itemRefs = useRef<Partial<Record<NavKey, HTMLButtonElement>>>({});

  useEffect(() => {
    const media = window.matchMedia(COMPACT_QUERY);
    const onResize = () => setCollapsed(media.matches);
    media.addEventListener("change", onResize);
    return () => media.removeEventListener("change", onResize);
  }, []);

  function onNavKeyDown(event: KeyboardEvent<HTMLElement>) {
    const index = ITEMS.findIndex(item => itemRefs.current[item.key] === event.target);
    if (index < 0) return;
    let next: number;
    switch (event.key) {
      case "ArrowDown": next = (index + 1) % ITEMS.length; break;
      case "ArrowUp": next = (index + ITEMS.length - 1) % ITEMS.length; break;
      case "Home": next = 0; break;
      case "End": next = ITEMS.length - 1; break;
      default: return;
    }
    event.preventDefault();
    itemRefs.current[ITEMS[next].key]?.focus();
  }

  return (
    <aside className="side-nav" data-collapsed={collapsed}>
      <div className="side-nav-head">
        <div className="side-nav-brand" title="Satelite">
          <span className="side-nav-mark" aria-hidden="true">◈</span>
          <span className="side-nav-brand-text">SATELITE</span>
        </div>
        <button
          type="button"
          className="side-nav-toggle icon-btn"
          aria-label={t("uiMenu.proMode")}
          title={t("uiMenu.proMode")}
          aria-expanded={!collapsed}
          aria-controls={navId}
          onClick={() => setCollapsed(value => !value)}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden="true" focusable="false">
            <path d="M4 4h16v16H4z M9 4v16" />
            <path d={collapsed ? "m12 9 3 3-3 3" : "m16 9-3 3 3 3"} />
          </svg>
        </button>
      </div>
      <nav id={navId} className="side-nav-items" aria-label={t("uiMenu.proMode")} onKeyDown={onNavKeyDown}>
        {ITEMS.map(item => (
          <button
            key={item.key}
            ref={element => { itemRefs.current[item.key] = element ?? undefined; }}
            type="button"
            className={`side-nav-item nav-item${active === item.key ? " active" : ""}`}
            aria-label={t(item.labelKey)}
            aria-current={active === item.key ? "page" : undefined}
            title={t(item.labelKey)}
            onClick={() => onChange(item.key)}
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">
              <path d={item.path} />
            </svg>
            <span className="side-nav-label">{t(item.labelKey)}</span>
          </button>
        ))}
      </nav>
      <div className="side-nav-tools">
        <ThemeSwitch />
        <UiModeMenu />
      </div>
    </aside>
  );
}
