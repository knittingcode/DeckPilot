/**
 * The device editor's Action Library sidebar: its width and whether it is
 * collapsed, kept per computer (localStorage). Pure helpers, so the rules
 * are tested without a browser.
 */

export const LIBRARY_WIDTH_KEY = "deckpilot.editor.libraryWidth"
export const LIBRARY_COLLAPSED_KEY = "deckpilot.editor.libraryCollapsed"

/** Width limits in CSS pixels. */
export const LIBRARY_WIDTH = { min: 224, default: 288, max: 480 }

/**
 * Below this window width the library leaves the layout and opens as an
 * overlay above the editor instead (CSS pixels).
 */
export const LIBRARY_OVERLAY_BELOW = 960

/** Keyboard step of the resize handle. */
export const LIBRARY_STEP = 16

export function clampLibraryWidth(width: number) {
  if (!Number.isFinite(width)) return LIBRARY_WIDTH.default
  return Math.round(Math.min(LIBRARY_WIDTH.max, Math.max(LIBRARY_WIDTH.min, width)))
}

type Storage = Pick<globalThis.Storage, "getItem" | "setItem">

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage
  } catch {
    return null
  }
}

export function loadLibraryWidth(store: Storage | null = storage()) {
  try {
    const raw = store?.getItem(LIBRARY_WIDTH_KEY)
    return raw ? clampLibraryWidth(Number(raw)) : LIBRARY_WIDTH.default
  } catch {
    return LIBRARY_WIDTH.default
  }
}

export function saveLibraryWidth(width: number, store: Storage | null = storage()) {
  try {
    store?.setItem(LIBRARY_WIDTH_KEY, String(clampLibraryWidth(width)))
  } catch {
    // Remembered for this session only.
  }
}

export function loadLibraryCollapsed(store: Storage | null = storage()) {
  try {
    return store?.getItem(LIBRARY_COLLAPSED_KEY) === "true"
  } catch {
    return false
  }
}

export function saveLibraryCollapsed(collapsed: boolean, store: Storage | null = storage()) {
  try {
    store?.setItem(LIBRARY_COLLAPSED_KEY, String(collapsed))
  } catch {
    // Remembered for this session only.
  }
}
