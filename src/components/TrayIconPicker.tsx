import { useI18n, type MessageKey } from "../i18n";
import type { TrayIconStyle } from "../types";
import trayBadge from "../../src-tauri/icons/tray/badge-on.png";
import trayBuddy from "../../src-tauri/icons/tray/buddy-on.png";
import trayGhost from "../../src-tauri/icons/tray/ghost-on.png";
import trayMark from "../../src-tauri/icons/tray/mark-on.png";

const TRAY_ICON_PICKS: {
  value: TrayIconStyle;
  src: string;
  labelKey: MessageKey;
}[] = [
  { value: "badge", src: trayBadge, labelKey: "settings.trayIconBadge" },
  { value: "mark", src: trayMark, labelKey: "settings.trayIconMark" },
  { value: "ghost", src: trayGhost, labelKey: "settings.trayIconGhost" },
  { value: "buddy", src: trayBuddy, labelKey: "settings.trayIconBuddy" },
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
