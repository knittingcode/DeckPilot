/**
 * DeckPilot's appearance is permanently dark: one design system, no light
 * theme, no theme setting, independent of the operating system's
 * appearance. The tokens are in `index.css`; native windows start with the
 * same background (`tauri.conf.json`, `src-tauri/src/windows.rs`) and
 * `index.html` paints it before the stylesheet loads, so nothing flashes
 * white.
 */
export const THEME = "dark"

/**
 * Where DeckPilot ≤ 0.9 kept the light/dark/system choice. It no longer
 * applies; `ThemeProvider` removes it once so nothing reads it later.
 */
export const LEGACY_THEME_STORAGE_KEY = "deckpilot.theme"
