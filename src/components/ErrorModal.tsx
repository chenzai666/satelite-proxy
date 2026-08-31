import { useEffect, useRef, useState } from "react";
import { useI18n } from "../i18n";
import { GlassButton } from "./GlassButton";

interface Props {
  message: string;
  onClose: () => void;
  /** Dialog caption; defaults to the generic "Error". */
  title?: string;
  /** Optional recovery action such as retrying a TUN switch after the
   * administrator prompt. The caller owns whether it closes the modal. */
  action?: { label: string; onClick: () => void };
}

/**
 * Error popup — the standard surface for exception reports (invoke failures,
 * core errors, load errors). Replaces the old banner/capsule strips: the full
 * message lives in a scrollable, selectable <pre> and carries a copy button,
 * because backend error strings are often long diagnostic blobs.
 *
 * Stacks above ordinary form modals (backdrop z-index 60 vs 50) so failures
 * raised while a dialog is open still paint on top. Clicking the veil closes
 * it; clicks inside are stopped so text selection never dismisses by accident.
 */
export function ErrorModal({ message, onClose, title, action }: Props) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  const copiedTimer = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (copiedTimer.current != null) window.clearTimeout(copiedTimer.current);
    },
    [],
  );

  async function onCopy() {
    try {
      await navigator.clipboard.writeText(message);
      setCopied(true);
      if (copiedTimer.current != null) window.clearTimeout(copiedTimer.current);
      copiedTimer.current = window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard unavailable — the <pre> stays selectable as the fallback.
    }
  }

  return (
    <div className="modal-backdrop error-modal-backdrop" onClick={onClose}>
      <div
        className="modal error-modal"
        role="dialog"
        aria-modal="true"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="modal-header">
          <h2>{title ?? t("common.error")}</h2>
          <button
            type="button"
            className="icon-btn"
            onClick={onClose}
            aria-label={t("common.close")}
          >
            ×
          </button>
        </header>
        <div className="modal-body">
          <pre className="error-modal-text">{message}</pre>
        </div>
        <footer className="modal-footer">
          <GlassButton onClick={() => void onCopy()}>
            {copied ? t("common.copied") : t("common.copy")}
          </GlassButton>
          {action && (
            <GlassButton variant="primary" onClick={action.onClick}>
              {action.label}
            </GlassButton>
          )}
          <GlassButton variant="primary" onClick={onClose}>
            {t("common.close")}
          </GlassButton>
        </footer>
      </div>
    </div>
  );
}
