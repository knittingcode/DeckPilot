import { invoke } from "@tauri-apps/api/core"

import { inTauri } from "@/lib/window-sync"

/**
 * The operating system's preferred languages, most preferred first. In the
 * app they come from the desktop process (`system_locales`, which reads the
 * macOS preferred-languages list, the Windows user UI languages and the
 * Linux locale variables); a plain browser and the tests use
 * `navigator.languages`. Never fails: an empty list means "unknown".
 */
export async function systemLocales(): Promise<string[]> {
  if (inTauri()) {
    try {
      const locales = await invoke<string[]>("system_locales")
      if (locales.length > 0) return locales
    } catch {
      // The webview's own list below.
    }
  }
  return browserLocales()
}

export function browserLocales(): string[] {
  if (typeof navigator === "undefined") return []
  // Not every environment has the list (older webviews, Node).
  const languages = (navigator.languages as readonly string[] | undefined) ?? []
  if (languages.length > 0) return [...languages]
  return typeof navigator.language === "string" && navigator.language ? [navigator.language] : []
}
