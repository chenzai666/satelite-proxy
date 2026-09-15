import { useLayoutEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

/** 菜单脱离侧栏滚动裁切层，避免底部规则集的菜单和二级菜单被截断。 */
export function RulesetMenu({ anchor, onClose, children }: {
  anchor: HTMLElement | null;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const menu = ref.current;
    if (!anchor || !menu) return;
    const zoom = Number.parseFloat(getComputedStyle(document.documentElement).zoom) || 1;
    const rect = anchor.getBoundingClientRect();
    const width = menu.offsetWidth;
    const height = menu.offsetHeight;
    const viewportHeight = (window.visualViewport?.height ?? window.innerHeight) / zoom;
    const viewportWidth = (window.visualViewport?.width ?? window.innerWidth) / zoom;
    const below = rect.bottom / zoom + 4;
    menu.style.left = `${Math.max(8, Math.min(rect.left / zoom, viewportWidth - width - 8))}px`;
    menu.style.top = `${Math.max(8, Math.min(below + height <= viewportHeight - 8 ? below : rect.top / zoom - height - 4, viewportHeight - height - 8))}px`;
    menu.style.visibility = "visible";
    const closeOnScroll = (event: Event) => {
      if (!(event.target instanceof Node) || !menu.contains(event.target)) onClose();
    };
    window.addEventListener("scroll", closeOnScroll, true);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("scroll", closeOnScroll, true);
      window.removeEventListener("resize", onClose);
    };
  }, [anchor, onClose]);
  return createPortal(<div ref={ref} data-ruleset-menu role="menu"
    className="rule-menu-pop ruleset-menu-pop ruleset-menu-portal"
    style={{ position: "fixed", inset: "auto", visibility: "hidden", zIndex: 1100 }}>
    {children}
  </div>, document.body);
}
