/**
 * Where the main window is: a workspace page (dashboard, editors, module
 * pages) or a settings page. Settings replace the workspace sidebar in the
 * same window; leaving them returns to the workspace page that was open
 * before.
 *
 * Navigation keeps a browser-style history with back and forward. The model
 * is pure so that it can be tested without React.
 */

/** Identifier of a settings page: a core page or `module:<module id>`. */
export type SettingsPageId = string

export type Location =
  { mode: "workspace"; page: string } | { mode: "settings"; page: SettingsPageId }

export interface NavigationState {
  /** Visited locations, oldest first. */
  entries: Location[]
  /** Position of the current location in `entries`. */
  index: number
  /** The workspace page settings return to (see `withWorkspace`). */
  workspace: string
}

/** Pages kept in the history; older entries are dropped. */
export const HISTORY_LIMIT = 50

export const DEFAULT_WORKSPACE_PAGE = "dashboard"
export const DEFAULT_SETTINGS_PAGE = "appearance"

export function initialNavigation(
  location: Location = { mode: "workspace", page: DEFAULT_WORKSPACE_PAGE },
  workspace = DEFAULT_WORKSPACE_PAGE,
): NavigationState {
  return {
    entries: [location],
    index: 0,
    workspace: location.mode === "workspace" ? location.page : workspace,
  }
}

export function current(state: NavigationState): Location {
  return state.entries[state.index] ?? { mode: "workspace", page: state.workspace }
}

function same(a: Location, b: Location) {
  return a.mode === b.mode && a.page === b.page
}

/**
 * The workspace page settings return to: the last workspace page at or
 * before the current entry, so that going back into an earlier settings
 * visit returns to the page that was open before that visit.
 */
function withWorkspace(state: NavigationState): NavigationState {
  const page = state.entries
    .slice(0, state.index + 1)
    .reverse()
    .find((entry) => entry.mode === "workspace")?.page
  return page && page !== state.workspace ? { ...state, workspace: page } : state
}

/** Opens a location; forward history after the current entry is discarded. */
export function push(state: NavigationState, location: Location): NavigationState {
  if (same(current(state), location)) return state
  const entries = [...state.entries.slice(0, state.index + 1), location].slice(-HISTORY_LIMIT)
  return withWorkspace({ ...state, entries, index: entries.length - 1 })
}

export function canGoBack(state: NavigationState) {
  return state.index > 0
}

export function canGoForward(state: NavigationState) {
  return state.index < state.entries.length - 1
}

export function back(state: NavigationState): NavigationState {
  return canGoBack(state) ? withWorkspace({ ...state, index: state.index - 1 }) : state
}

export function forward(state: NavigationState): NavigationState {
  return canGoForward(state) ? withWorkspace({ ...state, index: state.index + 1 }) : state
}

/** Opens settings; without a page, the settings page that was open last. */
export function openSettings(state: NavigationState, page?: SettingsPageId): NavigationState {
  const target =
    page ??
    state.entries
      .slice(0, state.index + 1)
      .reverse()
      .find((entry) => entry.mode === "settings")?.page ??
    DEFAULT_SETTINGS_PAGE
  return push(state, { mode: "settings", page: target })
}

/** Leaves settings and returns to the workspace page that was open before. */
export function closeSettings(state: NavigationState): NavigationState {
  return push(state, { mode: "workspace", page: state.workspace })
}

/**
 * Replaces locations that no longer exist, e.g. the page of a module that was
 * disabled, or a page removed by an update. History entries that point to
 * them are dropped.
 */
export function sanitize(
  state: NavigationState,
  exists: (location: Location) => boolean,
): NavigationState {
  const workspace = exists({ mode: "workspace", page: state.workspace })
    ? state.workspace
    : DEFAULT_WORKSPACE_PAGE
  const kept: Location[] = []
  let index = 0
  state.entries.forEach((entry, i) => {
    if (!exists(entry)) return
    const last = kept.at(-1)
    if (last && same(last, entry)) {
      if (i <= state.index) index = kept.length - 1
      return
    }
    kept.push(entry)
    if (i <= state.index) index = kept.length - 1
  })
  if (kept.length === 0) return initialNavigation({ mode: "workspace", page: workspace })
  if (
    kept.length === state.entries.length &&
    workspace === state.workspace &&
    index === state.index
  ) {
    return state
  }
  return withWorkspace({ entries: kept, index, workspace })
}

/** Former workspace pages that are now part of Settings › Devices. */
const MOVED_TO_DEVICES = new Set(["devices", "models"])

/** Translates a page reference from an event or older state. */
export function locationOf(reference: string): Location {
  if (reference === "settings") return { mode: "settings", page: DEFAULT_SETTINGS_PAGE }
  if (reference.startsWith("settings:")) {
    return { mode: "settings", page: reference.slice("settings:".length) }
  }
  // Connected devices and the device models moved from the workspace to
  // the settings.
  if (MOVED_TO_DEVICES.has(reference)) return { mode: "settings", page: "devices" }
  return { mode: "workspace", page: reference }
}

const STORAGE_KEY = "deckpilot.navigation"

function isLocation(value: unknown): value is Location {
  if (typeof value !== "object" || value === null) return false
  const { mode, page } = value as Record<string, unknown>
  return (mode === "workspace" || mode === "settings") && typeof page === "string"
}

/** The state remembered from the last session, if any. */
export function loadNavigation(storage: Pick<Storage, "getItem"> | null): NavigationState | null {
  try {
    const raw = storage?.getItem(STORAGE_KEY)
    if (!raw) return null
    const value = JSON.parse(raw) as Record<string, unknown>
    const entries = Array.isArray(value.entries)
      ? value.entries.filter(isLocation).map((entry) =>
          // Locations from versions that had Connected Devices in the workspace.
          entry.mode === "workspace" ? locationOf(entry.page) : entry,
        )
      : []
    if (entries.length === 0) return null
    const dropped = Math.max(0, entries.length - HISTORY_LIMIT)
    const kept = entries.slice(dropped)
    const stored = typeof value.index === "number" ? value.index : entries.length - 1
    const index = Math.min(Math.max(0, Math.trunc(stored) - dropped), kept.length - 1)
    const workspace =
      typeof value.workspace === "string" && !MOVED_TO_DEVICES.has(value.workspace)
        ? value.workspace
        : DEFAULT_WORKSPACE_PAGE
    return withWorkspace({ entries: kept, index, workspace })
  } catch {
    return null
  }
}

export function saveNavigation(storage: Pick<Storage, "setItem"> | null, state: NavigationState) {
  try {
    storage?.setItem(STORAGE_KEY, JSON.stringify(state))
  } catch {
    // Remembered for this session only.
  }
}

/** `localStorage`, or `null` where it is not available. */
export function browserStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage
  } catch {
    return null
  }
}
