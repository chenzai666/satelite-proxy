import { useLayoutEffect, type RefObject } from "react";

/** 菜单保留原 DOM 归属，沿用主题/点击外部行为；全部几何在同一视口坐标中计算。
 * 写入绝对定位前按实际缩放换算，避免 CSS zoom 下固定坐标偏移。
 */
export function useDashboardMenuPosition(root: RefObject<HTMLDivElement | null>, open: boolean, configOpen: boolean) {
  useLayoutEffect(() => {
    const owner = root.current;
    if (!open || !owner) return;
    let frame = 0;
    function position() {
      frame = 0;
      if (!owner?.isConnected) return;
      const bounds = { left: 8, top: 8, right: window.innerWidth - 8, bottom: window.innerHeight - 8 };
      // 超出祖先滚动/裁剪区的部分即使在窗口内也不可见。
      for (let parent = owner.parentElement; parent; parent = parent.parentElement) {
        const style = getComputedStyle(parent);
        const rect = parent.getBoundingClientRect();
        const sx = parent.offsetWidth ? rect.width / parent.offsetWidth : 1;
        const sy = parent.offsetHeight ? rect.height / parent.offsetHeight : 1;
        if (/(auto|scroll|hidden|clip)/.test(style.overflowX)) {
          bounds.left = Math.max(bounds.left, rect.left + parent.clientLeft * sx + 4);
          bounds.right = Math.min(bounds.right, rect.left + (parent.clientLeft + parent.clientWidth) * sx - 4);
        }
        if (/(auto|scroll|hidden|clip)/.test(style.overflowY)) {
          bounds.top = Math.max(bounds.top, rect.top + parent.clientTop * sy + 4);
          bounds.bottom = Math.min(bounds.bottom, rect.top + (parent.clientTop + parent.clientHeight) * sy - 4);
        }
      }
      const menus = owner.querySelectorAll<HTMLElement>(".dash-more-menu, .dash-more-submenu");
      for (const menu of menus) {
        const anchor = menu.parentElement;
        if (!anchor || !anchor.offsetWidth) continue;
        const anchorRect = anchor.getBoundingClientRect();
        const scale = anchorRect.width / anchor.offsetWidth || 1;
        menu.style.right = "auto";
        menu.style.bottom = "auto";
        menu.style.minWidth = "0";
        menu.style.width = `${Math.min(208, Math.max(1, (bounds.right - bounds.left) / scale))}px`;
        menu.style.maxHeight = `${Math.max(1, (bounds.bottom - bounds.top) / scale)}px`;
        const width = menu.offsetWidth * scale;
        const height = menu.offsetHeight * scale;
        const flyout = menu.classList.contains("dash-more-submenu");
        let x = anchorRect.left;
        let y = anchorRect.bottom + 6 * scale;
        if (flyout) {
          const fitsRight = anchorRect.right + 4 * scale + width <= bounds.right;
          const fitsLeft = anchorRect.left - 4 * scale - width >= bounds.left;
          const right = fitsRight || (!fitsLeft && bounds.right - anchorRect.right >= anchorRect.left - bounds.left);
          x = right ? anchorRect.right + 4 * scale : anchorRect.left - width - 4 * scale;
          y = anchorRect.top;
          anchor.dataset.menuSide = right ? "right" : "left";
        } else if (y + height > bounds.bottom && anchorRect.top - height - 6 * scale >= bounds.top) {
          y = anchorRect.top - height - 6 * scale;
        }
        x = Math.max(bounds.left, Math.min(x, bounds.right - width));
        y = Math.max(bounds.top, Math.min(y, bounds.bottom - height));
        // offsetParent 的 padding edge 是 absolute 坐标原点，补偿其边框。
        menu.style.left = `${(x - anchorRect.left) / scale - anchor.clientLeft}px`;
        menu.style.top = `${(y - anchorRect.top) / scale - anchor.clientTop}px`;
      }
    }
    function schedule() { if (!frame) frame = requestAnimationFrame(position); }
    position();
    const observer = new ResizeObserver(schedule);
    observer.observe(owner);
    for (let parent = owner.parentElement; parent; parent = parent.parentElement) observer.observe(parent);
    owner.querySelectorAll<HTMLElement>(".dash-more-menu, .dash-more-submenu").forEach(menu => observer.observe(menu));
    // 打开绝对定位菜单可能使祖先滚动条出现；下一帧用稳定后的 client 区复核。
    schedule();
    window.addEventListener("resize", schedule);
    document.addEventListener("scroll", schedule, true);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("resize", schedule);
      document.removeEventListener("scroll", schedule, true);
    };
  }, [root, open, configOpen]);
}
