import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/messages";
import type { NavKey } from "../types";
import { UiModeMenu } from "../ui/UiModeMenu";
import { ThemeSwitch } from "./ThemeSwitch";
import "./SideNav.css";

const COMPACT_QUERY = "(max-width: 1100px)";
const ITEMS: { key: NavKey; labelKey: MessageKey; path: string; activePath: string }[] = [
  { key: "dashboard", labelKey: "dashboard.title", path: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z", activePath: "M3 3h8v8H3z M14 3h7v7h-7z M3 14h7v7H3z M14 13h7v8h-7z" },
  { key: "nodes", labelKey: "nodes.title", path: "M8 5h8 M6 8v8 M18 8v8 M8 19h8 M3 3h5v5H3z M16 3h5v5h-5z M3 16h5v5H3z M16 16h5v5h-5z", activePath: "M2 2h7v7H2z M15 2h7v7h-7z M2 15h7v7H2z M15 15h7v7h-7z M9 4h6v2H9z M4 9h2v6H4z M18 9h2v6h-2z M9 18h6v2H9z" },
  { key: "config", labelKey: "config.title", path: "M3 6h7l2 2h9v12H3z M3 6V4h7l2 2h7v2", activePath: "M3 3h7l3 3h8v2H3z M2 9h20l-2 12H4z" },
  { key: "traffic", labelKey: "traffic.title", path: "M3 20h18 M6 16v-5 M12 16V4 M18 16V8", activePath: "M4 10h4v7H4z M10 3h4v14h-4z M16 7h4v10h-4z M3 19h18v2H3z" },
  { key: "logs", labelKey: "logs.title", path: "M5 3h14v18H5z M8 7h8 M8 12h8 M8 17h5", activePath: "M4 2h12l4 4v16H4z M8 7v2h8V7z M8 12v2h8v-2z M8 17v2h5v-2z" },
  { key: "settings", labelKey: "settings.title", path: "M4 6h4 M12 6h8 M4 12h10 M18 12h2 M4 18h2 M10 18h10 M8 3v6 M14 9v6 M6 15v6", activePath: "M6 2h4v8H6z M3 5h3v2H3z M10 5h11v2H10z M12 8h4v8h-4z M3 11h9v2H3z M16 11h5v2h-5z M4 14h4v8H4z M3 17h1v2H3z M8 17h13v2H8z" },
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
  const navRef = useRef<HTMLElement>(null);
  const indicatorRef = useRef<HTMLSpanElement>(null);
  const indicatorLayout = useRef<{
    active: NavKey; collapsed: boolean; navWidth: number; navHeight: number;
    x: number; y: number; width: number; height: number;
  } | null>(null);

  useLayoutEffect(() => {
    const nav = navRef.current;
    const indicator = indicatorRef.current;
    const selected = itemRefs.current[active];
    if (!nav || !indicator || !selected) return;

    const measure = (allowMotion: boolean, force = false) => {
      // nav 是 offsetParent；坐标属于滚动内容，不减 scrollTop。
      const next = {
        active, collapsed, navWidth: nav.clientWidth, navHeight: nav.clientHeight,
        x: selected.offsetLeft, y: selected.offsetTop,
        width: selected.offsetWidth, height: selected.offsetHeight,
      };
      const previous = indicatorLayout.current;
      if (!force && previous && Object.keys(next).every(key =>
        next[key as keyof typeof next] === previous[key as keyof typeof next])) return;
      const animate = allowMotion && document.documentElement.dataset.motionReduced !== "true"
        && previous && previous.active !== active
        && previous.collapsed === collapsed
        && previous.navWidth === next.navWidth && previous.navHeight === next.navHeight
        && previous.width === next.width && previous.height === next.height;
      indicator.dataset.motion = animate ? "true" : "false";
      indicator.style.width = `${next.width}px`;
      indicator.style.height = `${next.height}px`;
      indicator.style.transform = `translate(${next.x}px, ${next.y}px)`;
      nav.dataset.indicatorReady = "true";
      indicatorLayout.current = next;
    };

    measure(true);
    const observer = new ResizeObserver(() => measure(false));
    observer.observe(nav);
    // 字体、语言和折叠导致任一按钮尺寸变化时，立即重新对齐。
    for (const item of Object.values(itemRefs.current)) {
      if (item) observer.observe(item);
    }
    const onResize = () => measure(false);
    window.addEventListener("resize", onResize);
    // 模式或系统偏好变化时，取消在途过渡并直接吸附当前按钮。
    const motionObserver = new MutationObserver(() => measure(false, true));
    motionObserver.observe(document.documentElement, {
      attributes: true, attributeFilter: ["data-ui-motion", "data-motion-reduced"],
    });
    return () => {
      observer.disconnect();
      motionObserver.disconnect();
      window.removeEventListener("resize", onResize);
    };
  }, [active, collapsed]);

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
      <nav ref={navRef} id={navId} className="side-nav-items" aria-label={t("uiMenu.proMode")} onKeyDown={onNavKeyDown}>
        <span ref={indicatorRef} className="side-nav-indicator" aria-hidden="true" />
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
              <path
                d={active === item.key ? item.activePath : item.path}
                fill={active === item.key ? "currentColor" : "none"}
                stroke={active === item.key ? "none" : "currentColor"}
                fillRule="evenodd"
              />
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
