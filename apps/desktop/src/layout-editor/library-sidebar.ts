/**
 * Preferences of the device editor's Action Library, kept per computer
 * (localStorage). Pure helpers, so the rules are tested without a browser.
 *
 * The library has one fixed width and is always shown; the width and
 * collapsed state earlier versions remembered are removed once
 * (`OBSOLETE_LIBRARY_KEYS`).
 */

/** Where earlier versions remembered the library's width and collapsed state. */
export const OBSOLETE_LIBRARY_KEYS = [
  "deckpilot.editor.libraryWidth",
  "deckpilot.editor.libraryCollapsed",
] as const

type Storage = Pick<globalThis.Storage, "getItem" | "setItem" | "removeItem">

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage
  } catch {
    return null
  }
}

/** Forgets what earlier versions remembered about the library's size. */
export function removeObsoleteLibraryKeys(store: Storage | null = storage()) {
  for (const key of OBSOLETE_LIBRARY_KEYS) {
    try {
      store?.removeItem(key)
    } catch {
      // Nothing is read from it any more.
    }
  }
}

export const LIBRARY_COMPATIBLE_ONLY_KEY = "deckpilot.editor.compatibleOnly"

/** Whether the library hides what cannot go on the selected control. */
export function loadCompatibleOnly(store: Storage | null = storage()) {
  try {
    return store?.getItem(LIBRARY_COMPATIBLE_ONLY_KEY) === "true"
  } catch {
    return false
  }
}

export function saveCompatibleOnly(on: boolean, store: Storage | null = storage()) {
  try {
    store?.setItem(LIBRARY_COMPATIBLE_ONLY_KEY, String(on))
  } catch {
    // Remembered for this session only.
  }
}
