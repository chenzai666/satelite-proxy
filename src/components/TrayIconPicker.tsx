import { useI18n, type MessageKey } from "../i18n";
import type { TrayIconStyle } from "../types";
import trayBadge from "../../src-tauri/icons/tray/badge-on.png";
import trayBuddy from "../../src-tauri/icons/tray/buddy-on.png";
import trayGhost from "../../src-tauri/icons/tray/ghost-on.png";
import trayMark from "../../src-tauri/icons/tray/mark-on.png";
import trayDanger from "../../src-tauri/icons/tray/danger-on.png";
import trayDanger2 from "../../src-tauri/icons/tray/danger2-on.png";
import trayGhost2 from "../../src-tauri/icons/tray/ghost2-on.png";
import trayFaceid from "../../src-tauri/icons/tray/faceid-on.png";

const TRAY_ICON_PICKS: {
  value: TrayIconStyle;
  src: string;
  labelKey: MessageKey;
}[] = [
  { value: "badge", src: trayBadge, labelKey: "settings.trayIconBadge" },
  { value: "mark", src: trayMark, labelKey: "settings.trayIconMark" },
  { value: "ghost", src: trayGhost, labelKey: "settings.trayIconGhost" },
  { value: "ghost2", src: trayGhost2, labelKey: "settings.trayIconGhost2" },
  { value: "faceid", src: trayFaceid, labelKey: "settings.trayIconFaceid" },
  { value: "buddy", src: trayBuddy, labelKey: "settings.trayIconBuddy" },
  { value: "danger", src: trayDanger, labelKey: "settings.trayIconDanger" },
  { value: "danger2", src: trayDanger2, labelKey: "settings.trayIconDanger2" },
];

interface Props {
  value?: TrayIconStyle | null;
  disabled?: boolean;
  onChange: (value: TrayIconStyle) => void;
  "aria-label"?: string;
}

export function TrayIconPicker({
  value,
  disabled = false,
  onChange,
  "aria-label": ariaLabel,
}: Props) {
  const { t } = useI18n();
  const current = value ?? "badge";

  return (
    <div className="tray-icon-picks" role="group" aria-label={ariaLabel}>
      {TRAY_ICON_PICKS.map((opt) => {
        const active = current === opt.value;
        return (
          <button
            key={opt.value}
            type="button"
            className={`tray-icon-pick${active ? " active" : ""}`}
            title={t(opt.labelKey)}
            aria-label={t(opt.labelKey)}
            aria-pressed={active}
            disabled={disabled}
            onClick={() => onChange(opt.value)}
          >
            <img src={opt.src} alt="" draggable={false} />
          </button>
        );
      })}
    </div>
  );
}
