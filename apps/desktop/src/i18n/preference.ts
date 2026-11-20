import { isLanguage, type Language } from "./config"

/**
 * The language the user chose: `system` follows the operating system's
 * preferred languages (the default), or one language explicitly.
 *
 * It belongs to the app on this computer, not to a profile, and is kept
 * in the webview's `localStorage`, which every window of the app shares:
 * the unlock window needs it before the encrypted configuration is open,
 * so it cannot live in the configuration database (docs/i18n.md).
 */
export type LanguagePreference = "system" | Language

export const LANGUAGE_STORAGE_KEY = "deckpilot.language"

export const DEFAULT_LANGUAGE_PREFERENCE: LanguagePreference = "system"

export function isLanguagePreference(value: unknown): value is LanguagePreference {
  return value === "system" || isLanguage(value)
}

/** The webview's storage, if this environment has one that works. */
export function browserStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage
  } catch {
    return null
  }
}

/** The stored preference; `system` when none (or an unknown one) is stored. */
export function readLanguagePreference(
  storage: Pick<Storage, "getItem"> | null = browserStorage(),
): LanguagePreference {
  try {
    const value = storage?.getItem(LANGUAGE_STORAGE_KEY)
    return isLanguagePreference(value) ? value : DEFAULT_LANGUAGE_PREFERENCE
  } catch {
    return DEFAULT_LANGUAGE_PREFERENCE
  }
}

/** Stores the preference; `system` removes the entry, the default. */
export function writeLanguagePreference(
  preference: LanguagePreference,
  storage: Pick<Storage, "setItem" | "removeItem"> | null = browserStorage(),
) {
  try {
    if (preference === DEFAULT_LANGUAGE_PREFERENCE) storage?.removeItem(LANGUAGE_STORAGE_KEY)
    else storage?.setItem(LANGUAGE_STORAGE_KEY, preference)
  } catch {
    // This session keeps the language; the next one follows the system.
  }
}
