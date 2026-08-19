import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";

/** Movement (px) before a press turns into a drag; below it stays a click. */
const DRAG_THRESHOLD_PX = 5;
/** Per-frame interpolation toward the pointer (0..1). Lower = floatier. */
const FOLLOW_LERP = 0.35;
/** Sibling make-way animation when the drop gap moves (FLIP). */
const FLIP_MS = 200;
/** Edge bands of the list that trigger auto-scroll, and max speed px/frame. */
const EDGE_PX = 28;
const EDGE_SPEED_PX = 13;

export interface RulesetDragState {
  id: string;
  /** Insertion slot among the non-dragged items (0..count-1). */
  insertIndex: number;
  /** Height of the dragged card; drives the gap placeholder. */
  height: number;
}

function prefersReducedMotion() {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

interface Session {
  id: string;
  pointerId: number;
  startX: number;
  startY: number;
  startIds: string[];
  list: HTMLElement;
  host: HTMLElement;
  preview: HTMLElement;
  /** Preview origin (fixed left/top at pickup). */
  originX: number;
  originY: number;
  /** Smoothed offset from origin, applied via the CSS `translate` property. */
  dx: number;
  dy: number;
  /** Pointer-driven target offset. */
  targetDx: number;
  targetDy: number;
  pointerY: number;
  height: number;
  insertIndex: number;
  raf: number;
}

/**
 * Pointer-based sortable for the rule-set list.
 *
 * HTML5 DnD is unreliable in the Tauri WebView, so this hand-rolls the usual
 * card-drag UX: press anywhere on a card and move past DRAG_THRESHOLD_PX to
 * "lift" it (a fixed-position clone follows the pointer with a slight lag),
 * a highlighted gap opens at the live insertion slot, siblings slide aside
 * (FLIP), and releasing commits the new order through `onReorder`.
 * Esc / pointercancel / window blur aborts without changing anything.
 */
export function useRulesetDragSort<T extends { id: string }>(options: {
  items: T[];
  onReorder: (next: T[], startIds: string[]) => void;
}) {
  const { items, onReorder } = options;
  const itemsRef = useRef(items);
  itemsRef.current = items;
  const onReorderRef = useRef(onReorder);
  onReorderRef.current = onReorder;

  const [drag, setDrag] = useState<RulesetDragState | null>(null);

  /** Press registered, still below the movement threshold. */
  const pendingRef = useRef<{
    id: string;
    pointerId: number;
    startX: number;
    startY: number;
  } | null>(null);
  const sessionRef = useRef<Session | null>(null);
  /** id -> viewport top at the previous commit; baseline for FLIP. */
  const flipTopsRef = useRef<Map<string, number>>(new Map());
  /** FLIP runs while dragging and for the first render after it ends. */
  const wasDraggingRef = useRef(false);

  const endSessionRef = useRef<(commit: boolean) => void>(() => {});
  const tickRef = useRef<() => void>(() => {});

  const detachWindowListeners = useCallback(() => {
    window.removeEventListener("pointermove", onWindowPointerMove);
    window.removeEventListener("pointerup", onWindowPointerUp);
    window.removeEventListener("pointercancel", onWindowPointerCancel);
  }, []);

  const detachSessionListeners = useCallback(() => {
    window.removeEventListener("keydown", onSessionKeyDown);
    window.removeEventListener("blur", onSessionBlur);
  }, []);

  /** Insertion slot from live card midpoints; the dragged card is excluded. */
  const updateInsertIndex = (s: Session) => {
    const nodes = Array.from(
      s.list.querySelectorAll<HTMLElement>("[data-ruleset-id]"),
    ).filter((n) => n.dataset.rulesetId !== s.id);
    let idx = nodes.length;
    for (let i = 0; i < nodes.length; i++) {
      const rect = nodes[i].getBoundingClientRect();
      if (s.pointerY < rect.top + rect.height / 2) {
        idx = i;
        break;
      }
    }
    if (idx !== s.insertIndex) {
      s.insertIndex = idx;
      setDrag({ id: s.id, insertIndex: idx, height: s.height });
    }
  };

  const activate = (
    p: { id: string; pointerId: number; startX: number; startY: number },
    pointerY: number,
  ) => {
    const node = document.querySelector<HTMLElement>(
      `[data-ruleset-id="${CSS.escape(p.id)}"]`,
    );
    const list = node?.closest<HTMLElement>(".ruleset-list") ?? null;
    const fromIndex = itemsRef.current.findIndex((it) => it.id === p.id);
    if (!node || !list || fromIndex < 0) {
      pendingRef.current = null;
      return;
    }
    const rect = node.getBoundingClientRect();

    // The floating preview is a DOM clone living outside React's tree (React
    // keeps re-rendering the list during the drag). It sits inside a fixed
    // host that carries the list's classes so descendant-scoped styles
    // (.rules-route-list .ruleset-name etc.) keep applying to the clone.
    const host = document.createElement("div");
    host.className = `ruleset-drag-host ${Array.from(list.classList)
      .filter((c) => c !== "card")
      .join(" ")}`;
    Object.assign(host.style, {
      position: "fixed",
      left: `${rect.left}px`,
      top: `${rect.top}px`,
      width: `${rect.width}px`,
      zIndex: "1000",
      margin: "0",
      padding: "0",
      border: "none",
      background: "none",
      boxShadow: "none",
      display: "block",
      overflow: "visible",
      maxHeight: "none",
      pointerEvents: "none",
    } satisfies Partial<CSSStyleDeclaration>);
    const preview = node.cloneNode(true) as HTMLElement;
    preview.querySelectorAll(".rule-menu-pop").forEach((el) => el.remove());
    preview.removeAttribute("data-ruleset-id");
    preview.classList.add("ruleset-drag-preview");
    preview.style.pointerEvents = "none";
    host.appendChild(preview);
    document.body.appendChild(host);
    // "Lift" pop on the next frame so the pickup transition is visible.
    requestAnimationFrame(() => preview.classList.add("lifted"));
    document.body.classList.add("ruleset-dragging");

    const session: Session = {
      id: p.id,
      pointerId: p.pointerId,
      startX: p.startX,
      startY: p.startY,
      startIds: itemsRef.current.map((it) => it.id),
      list,
      host,
      preview,
      originX: rect.left,
      originY: rect.top,
      dx: 0,
      dy: 0,
      targetDx: 0,
      targetDy: 0,
      pointerY,
      height: rect.height,
      insertIndex: fromIndex,
      raf: 0,
    };
    pendingRef.current = null;
    sessionRef.current = session;
    window.addEventListener("keydown", onSessionKeyDown);
    window.addEventListener("blur", onSessionBlur);
    setDrag({ id: p.id, insertIndex: fromIndex, height: rect.height });
    session.raf = requestAnimationFrame(() => tickRef.current());
  };

  /** Quick fade-out used when a drag is aborted. */
  const fadeOutPreview = (s: Session) => {
    if (prefersReducedMotion()) {
      s.host.remove();
      return;
    }
    s.preview.style.transition = "none";
    const anim = s.preview.animate(
      [{ opacity: 1 }, { opacity: 0 }],
      { duration: 100, easing: "ease-out" },
    );
    const remove = () => s.host.remove();
    anim.onfinish = remove;
    anim.oncancel = remove;
  };

  /** Glide the preview onto the card's final slot, then remove it. */
  const settlePreview = (s: Session) => {
    if (prefersReducedMotion()) {
      s.host.remove();
      return;
    }
    const finish = () => s.host.remove();
    requestAnimationFrame(() => {
      // React has committed the reordered list by now; land on the real card.
      const node = document.querySelector<HTMLElement>(
        `[data-ruleset-id="${CSS.escape(s.id)}"]`,
      );
      if (!node) {
        finish();
        return;
      }
      const rect = node.getBoundingClientRect();
      const tdx = rect.left - s.originX;
      const tdy = rect.top - s.originY;
      s.preview.style.transition = "none";
      const anim = s.preview.animate(
        [
          {
            translate: `${s.dx}px ${s.dy}px`,
            transform: getComputedStyle(s.preview).transform || "none",
            opacity: 1,
            offset: 0,
          },
          {
            translate: `${tdx}px ${tdy}px`,
            transform: "scale(1)",
            opacity: 1,
            offset: 0.7,
          },
          {
            translate: `${tdx}px ${tdy}px`,
            transform: "scale(1)",
            opacity: 0,
            offset: 1,
          },
        ],
        { duration: 170, easing: "cubic-bezier(0.2, 0, 0, 1)" },
      );
      anim.onfinish = finish;
      anim.oncancel = finish;
    });
  };

  /** Swallow the click that follows pointerup so a drop selects nothing. */
  const swallowNextClick = () => {
    const swallow = (ev: MouseEvent) => {
      ev.preventDefault();
      ev.stopPropagation();
    };
    document.addEventListener("click", swallow, { capture: true, once: true });
    window.setTimeout(
      () => document.removeEventListener("click", swallow, { capture: true }),
      250,
    );
  };

  endSessionRef.current = (commit: boolean) => {
    const s = sessionRef.current;
    if (!s) return;
    sessionRef.current = null;
    pendingRef.current = null;
    cancelAnimationFrame(s.raf);
    document.body.classList.remove("ruleset-dragging");
    detachSessionListeners();
    setDrag(null);
    if (!commit) {
      fadeOutPreview(s);
      return;
    }
    const items = itemsRef.current;
    const dragged = items.find((it) => it.id === s.id);
    if (!dragged) {
      s.host.remove();
      return;
    }
    const others = items.filter((it) => it.id !== s.id);
    const next = [
      ...others.slice(0, s.insertIndex),
      dragged,
      ...others.slice(s.insertIndex),
    ];
    swallowNextClick();
    onReorderRef.current(next, s.startIds);
    settlePreview(s);
  };

  // rAF loop: eased follow, edge auto-scroll and insertion-slot recomputation.
  // Geometry shifts while the list scrolls, so the index is recomputed every
  // frame instead of only on pointer moves.
  tickRef.current = () => {
    const s = sessionRef.current;
    if (!s) return;
    if (prefersReducedMotion()) {
      s.dx = s.targetDx;
      s.dy = s.targetDy;
    } else {
      s.dx += (s.targetDx - s.dx) * FOLLOW_LERP;
      s.dy += (s.targetDy - s.dy) * FOLLOW_LERP;
    }
    s.host.style.translate = `${s.dx}px ${s.dy}px`;

    const listRect = s.list.getBoundingClientRect();
    if (s.pointerY < listRect.top + EDGE_PX) {
      const depth = Math.min(
        1,
        Math.max(0.25, (listRect.top + EDGE_PX - s.pointerY) / EDGE_PX),
      );
      s.list.scrollTop -= Math.round(EDGE_SPEED_PX * depth);
    } else if (s.pointerY > listRect.bottom - EDGE_PX) {
      const depth = Math.min(
        1,
        Math.max(0.25, (s.pointerY - (listRect.bottom - EDGE_PX)) / EDGE_PX),
      );
      s.list.scrollTop += Math.round(EDGE_SPEED_PX * depth);
    }
    updateInsertIndex(s);
    s.raf = requestAnimationFrame(() => tickRef.current());
  };

  const onWindowPointerMove = useCallback((e: PointerEvent) => {
    const s = sessionRef.current;
    if (s) {
      if (s.pointerId !== e.pointerId) return;
      e.preventDefault();
      s.targetDx = e.clientX - s.startX;
      s.targetDy = e.clientY - s.startY;
      s.pointerY = e.clientY;
      return;
    }
    const p = pendingRef.current;
    if (!p || p.pointerId !== e.pointerId) return;
    const dx = e.clientX - p.startX;
    const dy = e.clientY - p.startY;
    if (dx * dx + dy * dy < DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX) return;
    activate(p, e.clientY);
  }, []);

  const onWindowPointerUp = useCallback(
    (e: PointerEvent) => {
      const s = sessionRef.current;
      const p = pendingRef.current;
      if (s && s.pointerId === e.pointerId) {
        endSessionRef.current(true);
        detachWindowListeners();
      } else if (p && p.pointerId === e.pointerId) {
        // Never crossed the threshold: plain click, let it play out.
        pendingRef.current = null;
        detachWindowListeners();
      }
    },
    [detachWindowListeners],
  );

  const onWindowPointerCancel = useCallback(
    (e: PointerEvent) => {
      const s = sessionRef.current;
      const p = pendingRef.current;
      if (s && s.pointerId === e.pointerId) {
        endSessionRef.current(false);
        detachWindowListeners();
      } else if (p && p.pointerId === e.pointerId) {
        pendingRef.current = null;
        detachWindowListeners();
      }
    },
    [detachWindowListeners],
  );

  const onSessionKeyDown = useCallback((e: KeyboardEvent) => {
    if (e.key === "Escape" && sessionRef.current) {
      endSessionRef.current(false);
      detachWindowListeners();
    }
  }, []);

  const onSessionBlur = useCallback(() => {
    if (sessionRef.current || pendingRef.current) {
      endSessionRef.current(false);
      detachWindowListeners();
    }
  }, []);

  const attachWindowListeners = useCallback(() => {
    window.addEventListener("pointermove", onWindowPointerMove);
    window.addEventListener("pointerup", onWindowPointerUp);
    window.addEventListener("pointercancel", onWindowPointerCancel);
  }, [onWindowPointerMove, onWindowPointerUp, onWindowPointerCancel]);

  const onItemPointerDown = useCallback(
    (id: string, e: ReactPointerEvent<HTMLElement>) => {
      if (e.button !== 0) return;
      if (pendingRef.current || sessionRef.current) return;
      // Touch: whole-card dragging would fight list scrolling; the ⋮⋮ handle
      // (touch-action: none) stays the touch entry point.
      if (
        e.pointerType === "touch" &&
        !(e.target as HTMLElement).closest(".ruleset-drag")
      ) {
        return;
      }
      // Interactive children keep their own pointer behavior.
      if (
        (e.target as HTMLElement).closest(
          "button, a, input, select, textarea, [data-ruleset-menu]",
        )
      ) {
        return;
      }
      pendingRef.current = {
        id,
        pointerId: e.pointerId,
        startX: e.clientX,
        startY: e.clientY,
      };
      attachWindowListeners();
    },
    [attachWindowListeners],
  );

  // FLIP: when the drop gap moves (or the list restores after cancel), cards
  // that changed slots animate from their previous position instead of
  // snapping. Runs for every render but only animates during/just after a
  // drag, and always refreshes the baseline tops.
  useLayoutEffect(() => {
    const animate = (!!sessionRef.current || wasDraggingRef.current) &&
      !prefersReducedMotion();
    wasDraggingRef.current = !!drag;
    const tops = new Map<string, number>();
    for (const n of Array.from(
      document.querySelectorAll<HTMLElement>("[data-ruleset-id]"),
    )) {
      if (n.offsetHeight === 0) continue; // hidden drag source
      const id = n.dataset.rulesetId ?? "";
      const top = n.getBoundingClientRect().top;
      tops.set(id, top);
      const prev = flipTopsRef.current.get(id);
      if (!animate || prev === undefined || prev === top) continue;
      n.style.transition = "none";
      n.style.transform = `translateY(${prev - top}px)`;
      void n.offsetHeight; // commit the inverted position before animating
      n.style.transition = `transform ${FLIP_MS}ms cubic-bezier(0.2, 0, 0, 1)`;
      n.style.transform = "";
      const clear = () => {
        n.style.transition = "";
        n.style.transform = "";
      };
      n.addEventListener(
        "transitionend",
        (ev) => {
          if (ev.propertyName === "transform") clear();
        },
        { once: true },
      );
      window.setTimeout(clear, FLIP_MS + 60);
    }
    flipTopsRef.current = tops;
  });

  // Abandon any live drag when the page goes away.
  useEffect(
    () => () => {
      endSessionRef.current(false);
      detachWindowListeners();
    },
    [detachWindowListeners],
  );

  return { drag, onItemPointerDown };
}
