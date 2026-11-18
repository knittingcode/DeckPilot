// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { DATA_CHANGED_EVENT, EDITING_PROFILE_EVENT, WINDOW_INSTANCE } from "@/lib/window-sync"
import { api, resetApi } from "@/test/api-mock"
import { agentConnection, profiles } from "@/test/fixtures"
import { emitBackend, eventModule, resetBackend } from "@/test/tauri"

import { useAppData } from "./app-data"

vi.mock("@/lib/api", async () => {
  const mock = (await import("@/test/api-mock")).apiModule
  const real = await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return { ...real, ...mock }
})
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

beforeEach(() => {
  resetApi()
  resetBackend()
  eventModule.emit.mockClear()
  localStorage.clear()
  // Inside Tauri, where windows announce their changes.
  Object.assign(window, { __TAURI_INTERNALS__: {} })
  api.listProfiles.mockResolvedValue(profiles())
  api.listModules.mockResolvedValue([])
  api.agentStatus.mockResolvedValue(agentConnection())
  api.updateState.mockRejectedValue(new Error("no updater"))
})
afterEach(() => {
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
})

async function listening() {
  await waitFor(() => {
    expect(emitBackend(DATA_CHANGED_EVENT, { source: WINDOW_INSTANCE })).toBeGreaterThan(0)
  })
}

describe("app data across windows", () => {
  it("announces its changes and reloads on the other windows' announcements", async () => {
    const { result } = renderHook(() => useAppData())
    await listening()
    await waitFor(() => {
      expect(result.current.profiles.status).toBe("ready")
    })
    const loads = api.listProfiles.mock.calls.length
    const modules = api.listModules.mock.calls.length

    act(() => {
      result.current.refresh()
      result.current.setModules([])
    })
    expect(eventModule.emit).toHaveBeenCalledWith(DATA_CHANGED_EVENT, {
      source: WINDOW_INSTANCE,
    })
    expect(eventModule.emit).toHaveBeenCalledWith(DATA_CHANGED_EVENT, {
      modules: true,
      source: WINDOW_INSTANCE,
    })

    // Another window enabled a module.
    act(() => {
      emitBackend(DATA_CHANGED_EVENT, { modules: true, source: "settings-window" })
    })
    await waitFor(() => {
      expect(api.listModules.mock.calls.length).toBe(modules + 1)
    })
    await waitFor(() => {
      expect(api.listProfiles.mock.calls.length).toBeGreaterThan(loads + 1)
    })
  })

  it("follows another window's profile choice unless it refuses it", async () => {
    let accept = true
    const { result } = renderHook(() => useAppData({ acceptEditing: () => accept }))
    await listening()
    act(() => {
      result.current.setEditingProfileId("work")
    })
    expect(eventModule.emit).toHaveBeenLastCalledWith(EDITING_PROFILE_EVENT, {
      id: "work",
      source: WINDOW_INSTANCE,
    })
    act(() => {
      emitBackend(EDITING_PROFILE_EVENT, { id: "home", source: "settings-window" })
    })
    expect(result.current.editing).toBe("home")

    // Unsaved changes: refused, and the other window is told this one's choice.
    accept = false
    act(() => {
      emitBackend(EDITING_PROFILE_EVENT, { id: "work", source: "settings-window" })
    })
    expect(result.current.editing).toBe("home")
    expect(eventModule.emit).toHaveBeenLastCalledWith(EDITING_PROFILE_EVENT, {
      id: "home",
      source: WINDOW_INSTANCE,
    })
  })
})
