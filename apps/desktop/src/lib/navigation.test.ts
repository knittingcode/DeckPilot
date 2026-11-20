import { describe, expect, it } from "vitest"

import {
  HISTORY_LIMIT,
  closeSettings,
  current,
  initialNavigation,
  loadNavigation,
  locationOf,
  openSettings,
  push,
  sanitize,
  saveNavigation,
} from "./navigation"

const workspace = (page: string) => ({ mode: "workspace" as const, page })
const settings = (page: string) => ({ mode: "settings" as const, page })

function memoryStorage() {
  const items = new Map<string, string>()
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => {
      items.set(key, value)
    },
  }
}

describe("navigation", () => {
  it("returns from settings to the workspace page that was open before", () => {
    let state = push(initialNavigation(), workspace("workflows"))
    state = openSettings(state, "security")
    state = openSettings(state, "modules")
    expect(current(state)).toEqual(settings("modules"))
    state = closeSettings(state)
    expect(current(state)).toEqual(workspace("workflows"))
  })

  it("keeps a bounded list of recent locations, the newest current", () => {
    let state = push(initialNavigation(), workspace("actions"))
    state = openSettings(state, "updates")
    expect(current(state)).toEqual(settings("updates"))
    // Opening the location on screen again changes nothing.
    expect(push(state, settings("updates"))).toBe(state)
    expect(state.entries).toEqual([
      workspace("designer"),
      workspace("actions"),
      settings("updates"),
    ])
    for (let i = 0; i < HISTORY_LIMIT + 5; i++) state = push(state, workspace(`page-${i % 2}`))
    expect(state.entries).toHaveLength(HISTORY_LIMIT)
    expect(state.index).toBe(HISTORY_LIMIT - 1)
  })

  it("remembers the workspace page under the settings across sections", () => {
    let state = push(initialNavigation(), workspace("dashboard"))
    state = openSettings(state, "devices")
    state = openSettings(state, "agent")
    expect(state.workspace).toBe("dashboard")
    expect(current(closeSettings(state))).toEqual(workspace("dashboard"))
  })

  it("reopens settings where they were left", () => {
    let state = openSettings(initialNavigation(), "storage")
    state = closeSettings(state)
    expect(current(openSettings(state))).toEqual(settings("storage"))
    expect(current(openSettings(initialNavigation()))).toEqual(settings("profiles"))
  })

  it("does not record the same location twice in a row", () => {
    const state = push(push(initialNavigation(), workspace("actions")), workspace("actions"))
    expect(state.entries).toHaveLength(2)
  })

  it("keeps a bounded history", () => {
    let state = initialNavigation()
    for (let i = 0; i < HISTORY_LIMIT + 10; i++) state = push(state, workspace(`page-${i}`))
    expect(state.entries).toHaveLength(HISTORY_LIMIT)
    expect(state.index).toBe(HISTORY_LIMIT - 1)
  })

  it("drops locations that no longer exist, such as pages of disabled modules", () => {
    let state = push(initialNavigation(), workspace("projects/projects"))
    state = openSettings(state, "module:projects")
    state = push(state, workspace("projects/projects"))
    state = push(state, workspace("actions"))
    const cleaned = sanitize(
      state,
      (location) => !location.page.includes("projects") && location.page !== "unknown",
    )
    expect(cleaned.entries).toEqual([workspace("designer"), workspace("actions")])
    expect(current(cleaned)).toEqual(workspace("actions"))

    // The open page itself disappeared: the nearest earlier page is shown.
    const open = push(
      push(initialNavigation(), workspace("actions")),
      workspace("projects/projects"),
    )
    const fallback = sanitize(open, (location) => location.page !== "projects/projects")
    expect(current(fallback)).toEqual(workspace("actions"))
    expect(fallback.workspace).toBe("actions")

    // Unchanged state keeps its identity, so React does not re-render.
    expect(sanitize(cleaned, () => true)).toBe(cleaned)
  })

  it("translates page references from other windows and older versions", () => {
    expect(locationOf("projects/projects")).toEqual(workspace("projects/projects"))
    expect(locationOf("settings")).toEqual(settings("profiles"))
    expect(locationOf("settings:security")).toEqual(settings("security"))
    expect(locationOf("devices")).toEqual(settings("devices"))
    // The Device Models page moved into Settings › Devices.
    expect(locationOf("models")).toEqual(settings("devices"))
    expect(locationOf("designer")).toEqual(workspace("designer"))
    // Profiles are managed in the settings window.
    expect(locationOf("profiles")).toEqual(settings("profiles"))
  })

  it("persists the history across launches", () => {
    const storage = memoryStorage()
    let state = push(initialNavigation(), workspace("workflows"))
    state = openSettings(state, "backup")
    saveNavigation(storage, state)
    expect(loadNavigation(storage)).toEqual(state)
  })

  it("ignores stored state it cannot read", () => {
    const storage = memoryStorage()
    expect(loadNavigation(storage)).toBeNull()
    storage.setItem("deckpilot.navigation", "{not json")
    expect(loadNavigation(storage)).toBeNull()
    storage.setItem("deckpilot.navigation", JSON.stringify({ entries: [{ mode: "x" }] }))
    expect(loadNavigation(storage)).toBeNull()
    storage.setItem(
      "deckpilot.navigation",
      JSON.stringify({ entries: [workspace("actions")], index: 7, workspace: 3 }),
    )
    expect(loadNavigation(storage)).toEqual({
      entries: [workspace("actions")],
      index: 0,
      workspace: "actions",
    })
  })

  it("moves Connected Devices from a stored workspace page to the settings", () => {
    const storage = memoryStorage()
    storage.setItem(
      "deckpilot.navigation",
      JSON.stringify({
        entries: [workspace("actions"), workspace("devices")],
        index: 1,
        workspace: "devices",
      }),
    )
    const state = loadNavigation(storage)
    expect(state && current(state)).toEqual(settings("devices"))
    expect(state?.workspace).toBe("actions")
  })

  it("moves Device Models from a stored workspace page to the settings", () => {
    const storage = memoryStorage()
    storage.setItem(
      "deckpilot.navigation",
      JSON.stringify({
        entries: [workspace("designer"), workspace("models")],
        index: 1,
        workspace: "models",
      }),
    )
    const state = loadNavigation(storage)
    expect(state && current(state)).toEqual(settings("devices"))
    expect(state?.workspace).toBe("designer")
  })

  it("moves the Profiles page from a stored workspace page to the settings", () => {
    const storage = memoryStorage()
    storage.setItem(
      "deckpilot.navigation",
      JSON.stringify({
        entries: [workspace("actions"), workspace("profiles")],
        index: 1,
        workspace: "profiles",
      }),
    )
    const state = loadNavigation(storage)
    expect(state && current(state)).toEqual(settings("profiles"))
    // The main window keeps only workspace pages: without settings
    // locations the history ends at the Action Library.
    const main = state && sanitize(state, (location) => location.mode === "workspace")
    expect(main && current(main)).toEqual(workspace("actions"))
  })
})
