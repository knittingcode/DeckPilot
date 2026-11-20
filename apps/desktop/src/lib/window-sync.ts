/**
 * Keeps the app's windows in step. The main window and the settings window
 * each hold their own copy of the shared state (profiles, modules, the
 * profile being edited, the language); a change in one is announced to the
 * others with a Tauri event, which every window of the app receives. A
 * window ignores its own announcements.
 *
 * Outside Tauri (tests, a plain browser) there are no other windows and
 * nothing is sent or received.
 */

import { emit, listen } from "@tauri-apps/api/event"

/** Profiles, actions, layouts or modules changed (see `DataChanged`). */
export const DATA_CHANGED_EVENT = "deckpilot://data-changed"
/** Another profile is being edited (payload: `EditingChanged`). */
export const EDITING_PROFILE_EVENT = "deckpilot://editing-profile"
/** The language preference changed (payload: `LanguageChanged`). */
export const LANGUAGE_EVENT = "deckpilot://language"

interface Announcement {
  /** The window that sent it. */
  source: string
}

export interface DataChanged extends Announcement {
  /** Modules were enabled, disabled or configured. */
  modules?: boolean
}

export interface EditingChanged extends Announcement {
  id: string
}

export interface LanguageChanged extends Announcement {
  /** The new preference (`LanguagePreference`): `system`, `en` or `pl`. */
  preference: string
}

/** This window's identity in announcements. */
export const WINDOW_INSTANCE = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`

export function inTauri() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
}

/** What each announcement carries. */
export interface Announcements {
  [DATA_CHANGED_EVENT]: DataChanged
  [EDITING_PROFILE_EVENT]: EditingChanged
  [LANGUAGE_EVENT]: LanguageChanged
}

export type AnnouncementName = keyof Announcements

/** Tells the other windows; never fails. */
export function announce<K extends AnnouncementName>(
  event: K,
  payload: Omit<Announcements[K], "source">,
) {
  if (!inTauri()) return
  try {
    emit(event, { ...payload, source: WINDOW_INSTANCE }).catch(() => undefined)
  } catch {
    // Another window simply does not learn about it.
  }
}

/**
 * Calls `handler` for what other windows announce; returns the function
 * that stops listening.
 */
export function onAnnouncement<K extends AnnouncementName>(
  event: K,
  handler: (payload: Announcements[K]) => void,
): () => void {
  if (!inTauri()) return () => undefined
  let stopped = false
  let stop: (() => void) | undefined
  try {
    listen<Announcements[K]>(event, ({ payload }) => {
      if (!stopped && payload.source !== WINDOW_INSTANCE) handler(payload)
    }).then(
      (unlisten) => {
        if (stopped) unlisten()
        else stop = unlisten
      },
      () => undefined,
    )
  } catch {
    // Not listening: this window keeps its own state.
  }
  return () => {
    stopped = true
    stop?.()
  }
}
