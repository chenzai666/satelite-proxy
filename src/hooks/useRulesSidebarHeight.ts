import { useLayoutEffect, useRef } from "react";

/**
 * 侧栏只占当前可视区域；用实际几何尺寸兼容根节点 zoom 和窗口缩放。
 *
 * The value must not be recalculated from `.main` scroll events. The sidebar
 * is sticky, so its `top` changes while the page moves; using that changing
 * top to update `max-height` creates a feedback loop where the outer page
 * grows as it is scrolled and its scrollbar thumb keeps shrinking.
 */
export function useRulesSidebarHeight() {
  const ref = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const sidebar = ref.current;
    if (!sidebar) return;
    const main = sidebar.closest<HTMLElement>(".main");
    let frame = 0;
    const measure = () => {
      frame = 0;
      const zoom = Number.parseFloat(getComputedStyle(document.documentElement).zoom) || 1;
      const bottom = Math.min(window.visualViewport?.height ?? window.innerHeight,
        main?.getBoundingClientRect().bottom ?? Infinity);
      const height = Math.max(180, (bottom - sidebar.getBoundingClientRect().top) / zoom - 12);
      const value = `${Math.floor(height)}px`;
      if (sidebar.style.maxHeight !== value) sidebar.style.maxHeight = value;
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(measure); };
    const observer = new ResizeObserver(schedule);
    observer.observe(main ?? document.documentElement);
    window.addEventListener("resize", schedule);
    window.visualViewport?.addEventListener("resize", schedule);
    measure();
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", schedule);
      window.visualViewport?.removeEventListener("resize", schedule);
    };
  }, []);
  return ref;
}
