import { useTheme } from "../theme";

/**
 * Compact day/night capsule (☼ ◐) used in the navbar tools group. Shared by the
 * full and simple UI modes so their toolbars stay visually aligned.
 */
export function ThemeSwitch() {
  const { theme, setTheme } = useTheme();
  return (
    <div
      className="topnav-theme-switch"
      role="group"
      aria-label="外观"
    >
      <button
        type="button"
        className={`topnav-theme-btn ${theme === "day" ? "active" : ""}`}
        aria-label="亮色模式"
        aria-pressed={theme === "day"}
        title="Day"
        onClick={() => void setTheme("day")}
      >
        ☼
      </button>
      <button
        type="button"
        className={`topnav-theme-btn ${theme === "aerospace" ? "active" : ""}`}
        aria-label="暗色模式"
        aria-pressed={theme === "aerospace"}
        title="Mission"
        onClick={() => void setTheme("aerospace")}
      >
        ☾
      </button>
    </div>
  );
}
