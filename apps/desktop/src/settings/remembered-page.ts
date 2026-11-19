import { DEFAULT_SETTINGS_PAGE } from "@/lib/navigation"

export const SETTINGS_PAGE_KEY = "deckpilot.settings.page"

/** The settings page shown last, in this or an earlier session. */
export function rememberedSettingsPage(storage: Pick<Storage, "getItem"> | null): string {
  try {
    return storage?.getItem(SETTINGS_PAGE_KEY) ?? DEFAULT_SETTINGS_PAGE
  } catch {
    return DEFAULT_SETTINGS_PAGE
  }
}

/** Remembers the settings page on screen, for the next time they open. */
export function rememberSettingsPage(storage: Pick<Storage, "setItem"> | null, page: string) {
  try {
    storage?.setItem(SETTINGS_PAGE_KEY, page)
  } catch {
    // Opens at the last page of this session's history, or the default.
  }
}
