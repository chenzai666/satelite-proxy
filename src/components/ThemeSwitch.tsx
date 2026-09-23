import { useTheme } from "../theme";
import { useI18n } from "../i18n";

/**
 * Compact day/night capsule (☼ ◐) used in the navbar tools group. Shared by the
 * full and simple UI modes so their toolbars stay visually aligned.
 */
export function ThemeSwitch() {
  const { theme, setTheme } = useTheme();
  const { t } = useI18n();
  return (
    <div
      className="topnav-theme-switch"
      role="group"
      aria-label={t("theme.appearanceAria")}
    >
      <button
        type="button"
        className={`topnav-theme-btn ${theme === "day" ? "active" : ""}`}
        aria-label={t("theme.lightAria")}
        aria-pressed={theme === "day"}
        title="Day"
        onClick={() => void setTheme("day")}
      >
        ☼
      </button>
      <button
        type="button"
        className={`topnav-theme-btn ${theme === "aerospace" ? "active" : ""}`}
        aria-label={t("theme.darkAria")}
        aria-pressed={theme === "aerospace"}
        title="Mission"
        onClick={() => void setTheme("aerospace")}
      >
        ☾
      </button>
    </div>
  );
}
