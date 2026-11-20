// The languages and namespaces of DeckPilot's own interface (docs/i18n.md).

/** Languages with a complete translation; the first is the fallback. */
export const SUPPORTED_LANGUAGES = ["en", "pl"] as const

export type Language = (typeof SUPPORTED_LANGUAGES)[number]

/** Shown when no preferred language is supported, and for missing keys. */
export const FALLBACK_LANGUAGE: Language = "en"

/** One resource file per namespace and language: `locales/<lng>/<ns>.json`. */
export const NAMESPACES = [
  "common",
  "navigation",
  "settings",
  "devices",
  "layouts",
  "editor",
  "actions",
  "workflows",
  "workspace",
  "marketplace",
  "plugins",
  "permissions",
  "dialogs",
  "security",
  "notifications",
  "errors",
] as const

export type Namespace = (typeof NAMESPACES)[number]

/**
 * Each language's name in that language, as the language choice shows it
 * (a user who cannot read the current language still finds their own).
 */
export const LANGUAGE_NAMES: Record<Language, string> = {
  en: "English",
  pl: "Polski",
}

export function isLanguage(value: unknown): value is Language {
  return typeof value === "string" && (SUPPORTED_LANGUAGES as readonly string[]).includes(value)
}
