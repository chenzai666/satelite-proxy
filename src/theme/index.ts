export {
  ThemeProvider,
  useTheme,
  normalizeTheme,
  normalizeHeroStyle,
  applyThemeToDom,
  readStoredTheme,
  readStoredAccent,
} from "./ThemeContext";

export {
  ACCENTS,
  DEFAULT_ACCENT,
  defaultAccent,
  resolveAccent,
  isValidAccent,
  applyAccentToDom,
  type AccentPreset,
} from "./accents";
