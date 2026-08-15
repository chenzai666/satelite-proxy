import { useLayoutEffect, useMemo, useRef, useState } from "react";

interface VirtualRangeOptions {
  itemCount: number;
  itemSize: number;
  itemsPerRow?: number;
  enabled?: boolean;
  overscanRows?: number;
}

interface RowRange {
  startRow: number;
  endRow: number;
}

/** Window a fixed-height list against the app's existing scroll container. */
export function useVirtualRange({
  itemCount,
  itemSize,
  itemsPerRow = 1,
  enabled = true,
  overscanRows = 6,
}: VirtualRangeOptions) {
  const containerRef = useRef<HTMLElement | null>(null);
  const totalRows = Math.ceil(itemCount / itemsPerRow);
  const [rows, setRows] = useState<RowRange>(() => ({
    startRow: 0,
    endRow: Math.min(totalRows, 30),
  }));

  useLayoutEffect(() => {
    const container = containerRef.current;
    if (!enabled || !container) {
      setRows({ startRow: 0, endRow: totalRows });
      return;
    }

    const scroller = container.closest<HTMLElement>(".main");
    if (!scroller) {
      setRows({ startRow: 0, endRow: totalRows });
      return;
    }

    let frame = 0;
    const update = () => {
      frame = 0;
      const containerRect = container.getBoundingClientRect();
      const scrollerRect = scroller.getBoundingClientRect();
      const visibleTop = Math.max(0, scrollerRect.top - containerRect.top);
      const visibleBottom = Math.max(
        visibleTop,
        Math.min(containerRect.height, scrollerRect.bottom - containerRect.top),
      );
      const startRow = Math.max(
        0,
        Math.floor(visibleTop / itemSize) - overscanRows,
      );
      const endRow = Math.min(
        totalRows,
        Math.ceil(visibleBottom / itemSize) + overscanRows,
      );
      setRows((current) =>
        current.startRow === startRow && current.endRow === endRow
          ? current
          : { startRow, endRow },
      );
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(update);
    };

    update();
    scroller.addEventListener("scroll", schedule, { passive: true });
    window.addEventListener("resize", schedule);
    const observer = new ResizeObserver(schedule);
    observer.observe(scroller);
    observer.observe(container);
    return () => {
      if (frame) cancelAnimationFrame(frame);
      observer.disconnect();
      scroller.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
    };
  }, [enabled, itemSize, overscanRows, totalRows]);

  return useMemo(() => {
    if (!enabled) {
      return {
        containerRef,
        start: 0,
        end: itemCount,
        paddingTop: 0,
        paddingBottom: 0,
      };
    }
    const startRow = Math.min(rows.startRow, totalRows);
    const endRow = Math.max(startRow, Math.min(rows.endRow, totalRows));
    return {
      containerRef,
      start: startRow * itemsPerRow,
      end: Math.min(itemCount, endRow * itemsPerRow),
      paddingTop: startRow * itemSize,
      paddingBottom: Math.max(0, (totalRows - endRow) * itemSize),
    };
  }, [enabled, itemCount, itemSize, itemsPerRow, rows, totalRows]);
}
