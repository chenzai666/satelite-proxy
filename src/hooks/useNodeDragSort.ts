import { useEffect, useRef, type PointerEvent } from "react";

/** Pointer events work in WebView2/WKWebView and keep virtual lists enabled. */
export function useNodeDragSort(onDrop: (id: string, target: string, after: boolean) => void) {
  const callback = useRef(onDrop);
  callback.current = onDrop;
  const cleanup = useRef<() => void>(() => {});
  const suppressClick = useRef(false);
  useEffect(() => () => cleanup.current(), []);

  function onPointerDown(event: PointerEvent<HTMLElement>, id: string, enabled: boolean) {
    suppressClick.current = false;
    if (!enabled || event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey ||
      (event.target as HTMLElement).closest("button,input,select,textarea,a,[role=button]")) return;
    cleanup.current();
    const origin = event.currentTarget;
    const scroller = origin.closest<HTMLElement>(".main");
    const startX = event.clientX, startY = event.clientY, pointerId = event.pointerId;
    const zoom = Number.parseFloat(getComputedStyle(document.documentElement).zoom) || 1;
    let x = startX, y = startY, active = false, frame = 0;
    let preview: HTMLElement | null = null;
    let target: HTMLElement | null = null;
    let after = false;
    const oldSelect = document.body.style.userSelect;
    const oldCursor = document.body.style.cursor;
    function markTarget() {
      target?.classList.remove("node-drop-before", "node-drop-after");
      target = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-node-id]") ?? null;
      if (target?.dataset.nodeId === id) target = null;
      if (target) {
        const rect = target.getBoundingClientRect();
        after = target.classList.contains("node-card") ? x > rect.left + rect.width / 2 : y > rect.top + rect.height / 2;
        target.classList.add(after ? "node-drop-after" : "node-drop-before");
      }
    }
    function tick() {
      if (!active) return;
      if (preview) preview.style.transform = `translate(${(x - startX) / zoom}px, ${(y - startY) / zoom}px)`;
      if (scroller) {
        const rect = scroller.getBoundingClientRect();
        const delta = y < rect.top + 36 ? -12 : y > rect.bottom - 36 ? 12 : 0;
        if (delta) scroller.scrollTop += delta;
      }
      markTarget();
      frame = requestAnimationFrame(tick);
    }
    function move(e: globalThis.PointerEvent) {
      if (e.pointerId !== pointerId) return;
      x = e.clientX; y = e.clientY;
      if (!active && Math.hypot(x - startX, y - startY) >= 5) {
        active = true;
        suppressClick.current = true;
        const rect = origin.getBoundingClientRect();
        preview = origin.cloneNode(true) as HTMLElement;
        preview.removeAttribute("data-node-id");
        preview.setAttribute("aria-hidden", "true");
        Object.assign(preview.style, {
          position: "fixed", top: `${rect.top / zoom}px`, left: `${rect.left / zoom}px`,
          width: `${rect.width / zoom}px`, height: `${rect.height / zoom}px`, margin: "0",
          pointerEvents: "none", zIndex: "10000", opacity: ".85",
        });
        document.body.append(preview);
        document.body.style.userSelect = "none";
        document.body.style.cursor = "grabbing";
        tick();
      }
      if (active) e.preventDefault();
    }
    function finish(commit: boolean) {
      const targetId = target?.dataset.nodeId;
      const shouldCommit = commit && active && targetId;
      active = false;
      cancelAnimationFrame(frame);
      preview?.remove();
      target?.classList.remove("node-drop-before", "node-drop-after");
      document.body.style.userSelect = oldSelect;
      document.body.style.cursor = oldCursor;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("blur", cancel);
      window.removeEventListener("resize", cancel);
      window.removeEventListener("keydown", key);
      cleanup.current = () => {};
      if (shouldCommit) callback.current(id, targetId, after);
    }
    function up(e: globalThis.PointerEvent) { if (e.pointerId === pointerId) { x=e.clientX; y=e.clientY; if (active) markTarget(); finish(true); } }
    function cancel() { finish(false); }
    function key(e: KeyboardEvent) { if (e.key === "Escape") { e.preventDefault(); cancel(); } }
    window.addEventListener("pointermove", move, { passive: false });
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("blur", cancel);
    window.addEventListener("resize", cancel);
    window.addEventListener("keydown", key);
    cleanup.current = cancel;
  }
  return { onPointerDown, suppressClick };
}
