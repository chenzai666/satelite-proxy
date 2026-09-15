import { useLayoutEffect, type RefObject } from "react";

/**
 * Make a nested scroll region own every vertical input while its pointer or
 * focus is inside it. `overscroll-behavior` handles most Chromium cases, but
 * WebView can still hand a wheel gesture to the nearest ancestor when the
 * nested region reaches an edge. That made the rule-set scrollbar and the
 * page scrollbar move together.
 */
export function useIsolatedScroll(ref: RefObject<HTMLElement | null>): void {
  useLayoutEffect(() => {
    const scroller = ref.current;
    if (!scroller) return;

    const maxScrollTop = () => Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    const scrollBy = (delta: number) => {
      if (!Number.isFinite(delta) || delta === 0) return;
      scroller.scrollTop = Math.max(0, Math.min(maxScrollTop(), scroller.scrollTop + delta));
    };
    const wheelDelta = (event: WheelEvent) => {
      if (event.deltaMode === WheelEvent.DOM_DELTA_LINE) return event.deltaY * 16;
      if (event.deltaMode === WheelEvent.DOM_DELTA_PAGE) return event.deltaY * scroller.clientHeight;
      return event.deltaY;
    };
    const onWheel = (event: WheelEvent) => {
      // Always consume a vertical wheel gesture here, including at both
      // boundaries. Otherwise Chromium/WebView scroll chaining moves `.main`.
      if (event.deltaY === 0) return;
      event.preventDefault();
      event.stopPropagation();
      scrollBy(wheelDelta(event));
    };
    const onKeyDown = (event: KeyboardEvent) => {
      // Buttons inside a rule card keep their normal keyboard behavior.
      if (event.target !== scroller) return;
      const page = Math.max(1, scroller.clientHeight - 32);
      const delta =
        event.key === "ArrowDown" ? 40 :
        event.key === "ArrowUp" ? -40 :
        event.key === "PageDown" || event.key === " " ? page :
        event.key === "PageUp" ? -page :
        event.key === "Home" ? -Infinity :
        event.key === "End" ? Infinity :
        null;
      if (delta === null) return;
      event.preventDefault();
      event.stopPropagation();
      if (delta === Infinity) scroller.scrollTop = maxScrollTop();
      else if (delta === -Infinity) scroller.scrollTop = 0;
      else scrollBy(delta);
    };

    let touchY: number | null = null;
    const onTouchStart = (event: TouchEvent) => {
      touchY = event.touches.length === 1 ? event.touches[0].clientY : null;
    };
    const onTouchMove = (event: TouchEvent) => {
      if (touchY === null || event.touches.length !== 1) return;
      const nextY = event.touches[0].clientY;
      event.preventDefault();
      event.stopPropagation();
      scrollBy(touchY - nextY);
      touchY = nextY;
    };
    const clearTouch = () => { touchY = null; };

    scroller.addEventListener("wheel", onWheel, { passive: false });
    scroller.addEventListener("keydown", onKeyDown);
    scroller.addEventListener("touchstart", onTouchStart, { passive: true });
    scroller.addEventListener("touchmove", onTouchMove, { passive: false });
    scroller.addEventListener("touchend", clearTouch, { passive: true });
    scroller.addEventListener("touchcancel", clearTouch, { passive: true });
    return () => {
      scroller.removeEventListener("wheel", onWheel);
      scroller.removeEventListener("keydown", onKeyDown);
      scroller.removeEventListener("touchstart", onTouchStart);
      scroller.removeEventListener("touchmove", onTouchMove);
      scroller.removeEventListener("touchend", clearTouch);
      scroller.removeEventListener("touchcancel", clearTouch);
    };
  }, [ref]);
}
