import { DEFAULT_SETTINGS_PAGE, locationOf } from "@/lib/navigation"

export const SETTINGS_PAGE_KEY = "deckpilot.settings.page"

/**
 * The settings page a reference asks for: a settings page id
 * (`security`), `settings:<page>`, or a page that moved to the settings
 * (`profiles`, `models`). `null` for pages of the main window.
 */
export function settingsPageOf(reference: string): string | null {
  const location = locationOf(reference)
  if (location.mode === "settings") return location.page
  // Workspace pages: the core ones and module pages (`<module>/<page>`).
  if (WORKSPACE_PAGES.has(reference) || reference.includes("/")) return null
  return reference
}

const WORKSPACE_PAGES = new Set(["designer", "dashboard", "actions", "workflows", "marketplace"])

/** The page the window opens at: the one asked for, else the last one. */
export function initialSettingsPage(
  search: string,
  storage: Pick<Storage, "getItem"> | null,
): string {
  const asked = new URLSearchParams(search).get("page")
  const page = asked ? settingsPageOf(asked) : null
  if (page) return page
  try {
    return storage?.getItem(SETTINGS_PAGE_KEY) ?? DEFAULT_SETTINGS_PAGE
  } catch {
    return DEFAULT_SETTINGS_PAGE
  }
}

/** Remembers the page the settings window shows, for its next opening. */
export function rememberSettingsPage(storage: Pick<Storage, "setItem"> | null, page: string) {
  try {
    storage?.setItem(SETTINGS_PAGE_KEY, page)
  } catch {
    // Opens at the default page next time.
  }
}
