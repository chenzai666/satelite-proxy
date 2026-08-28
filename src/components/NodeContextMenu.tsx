import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useI18n } from "../i18n";
import type { ProxyNode } from "../types";

export interface NodeContextMenuState {
  node: ProxyNode;
  x: number;
  y: number;
}

interface Props {
  state: NodeContextMenuState | null;
  onClose: () => void;
  onEdit: (node: ProxyNode) => void;
  onCopyLink: (node: ProxyNode) => void;
  onShowQr: (node: ProxyNode) => void;
  onDelete: (node: ProxyNode) => void;
}

/** Local node actions only; the surrounding document suppresses the native menu. */
export function NodeContextMenu({
  state,
  onClose,
  onEdit,
  onCopyLink,
  onShowQr,
  onDelete,
}: Props) {
  const { t } = useI18n();
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!state) return;
    const closeOutside = (event: PointerEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) onClose();
    };
    const closeEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("pointerdown", closeOutside);
    window.addEventListener("keydown", closeEscape);
    return () => {
      window.removeEventListener("pointerdown", closeOutside);
      window.removeEventListener("keydown", closeEscape);
    };
  }, [state, onClose]);

  if (!state) return null;
  const x = Math.min(state.x, window.innerWidth - 220);
  const y = Math.min(state.y, window.innerHeight - 150);
  const node = state.node;

  return createPortal(
    <div
      ref={menuRef}
      className="node-context-menu"
      role="menu"
      aria-label={node.name}
      style={{ left: Math.max(8, x), top: Math.max(8, y) }}
    >
      <div className="node-context-menu-title" title={node.name}>{node.name}</div>
      <button
        type="button"
        className="node-context-menu-item"
        role="menuitem"
        onClick={() => {
          onEdit(node);
          onClose();
        }}
      >
        <span aria-hidden>✎</span>
        {t("nodes.contextEdit")}
      </button>
      <button
        type="button"
        className="node-context-menu-item"
        role="menuitem"
        onClick={() => {
          onCopyLink(node);
          onClose();
        }}
      >
        <span aria-hidden>⧉</span>
        {t("nodes.contextCopyLink")}
      </button>
      <button
        type="button"
        className="node-context-menu-item"
        role="menuitem"
        onClick={() => {
          onShowQr(node);
          onClose();
        }}
      >
        <span aria-hidden>▦</span>
        {t("nodes.contextShowQr")}
      </button>
      <div className="node-context-menu-divider" />
      <button
        type="button"
        className="node-context-menu-item danger"
        role="menuitem"
        onClick={() => {
          onDelete(node);
          onClose();
        }}
      >
        <span aria-hidden>×</span>
        {t("nodes.contextDelete")}
      </button>
    </div>,
    document.body,
  );
}
