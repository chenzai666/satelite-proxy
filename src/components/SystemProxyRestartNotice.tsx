import { useEffect } from "react";
import { useI18n } from "../i18n";
import { GlassButton } from "./GlassButton";

interface Props {
  open: boolean;
  onClose: () => void;
}

/** Shown only after a user-initiated switch into system-proxy capture. */
export function SystemProxyRestartNotice({ open, onClose }: Props) {
  const { t } = useI18n();

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, open]);

  if (!open) return null;

  return (
    <div className="modal-backdrop system-proxy-restart-backdrop">
      <div
        className="modal system-proxy-restart-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="system-proxy-restart-title"
        aria-describedby="system-proxy-restart-description"
      >
        <header className="modal-header">
          <h2 id="system-proxy-restart-title">
            {t("dashboard.restartAppsTitle")}
          </h2>
          <button
            type="button"
            className="icon-btn"
            onClick={onClose}
            aria-label={t("common.close")}
          >
            ×
          </button>
        </header>
        <div className="modal-body system-proxy-restart-body">
          <p id="system-proxy-restart-description">
            {t("dashboard.restartAppsNotice")}
          </p>
          <p className="hint">{t("dashboard.restartAppsDetail")}</p>
          <div className="modal-footer">
            <GlassButton variant="primary" onClick={onClose} autoFocus>
              {t("dashboard.restartAppsAcknowledge")}
            </GlassButton>
          </div>
        </div>
      </div>
    </div>
  );
}
