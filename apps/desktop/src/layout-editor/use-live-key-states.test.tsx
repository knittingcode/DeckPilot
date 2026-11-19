// @vitest-environment jsdom
import { act, cleanup, render, renderHook, screen, waitFor } from "@testing-library/react"
import "@testing-library/jest-dom/vitest"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { AgentConnection, KeyStateEntry, LivePreview } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { agentConnection, agentStatus } from "@/test/fixtures"
import { emitBackend, resetBackend } from "@/test/tauri"

import { LiveValuesNote } from "./live-values-note"
import {
  KEY_STATES_EVENT,
  type LiveStatus,
  liveValuesText,
  livePreview,
  mergeStates,
  useLiveKeyStates,
} from "./use-live-key-states"
import { usePreviewFaces } from "./use-preview-faces"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

const cpu = (detail: string): KeyStateEntry => ({
  action: "cpu",
  snapshot: { type: "ready", state: { detail, indicator: "ok", progress: 23 } },
})
const memory: KeyStateEntry = {
  action: "memory",
  snapshot: { type: "ready", state: { detail: "12.4/32 GiB" } },
}

/** A current agent that can watch key states. */
function agent(change: Partial<AgentConnection> = {}): AgentConnection {
  return agentConnection({ capabilities: ["keyStates"], ...change })
}

function watches() {
  return api.watchKeyStates.mock.calls.map(([profile, actions]) => [profile, actions])
}

beforeEach(() => {
  resetApi()
  resetBackend()
  let number = 0
  api.watchKeyStates.mockImplementation(() => Promise.resolve(++number))
})

afterEach(() => {
  cleanup()
})

describe("live values in words", () => {
  it("merges changes by action and draws stale values only when known", () => {
    expect(mergeStates([cpu("1%"), memory], [cpu("2%")])).toEqual([cpu("2%"), memory])
    expect(livePreview("live", [])).toBeNull()
    expect(livePreview("live", [memory])).toEqual({ connected: true, states: [memory] })
    expect(livePreview("disconnected", [memory])).toEqual({ connected: false, states: [memory] })
    expect(livePreview("stopped", [memory])?.connected).toBe(false)
    expect(livePreview("inactiveProfile", [memory])).toBeNull()
    expect(livePreview("unsupported", [memory])).toBeNull()
  })

  it("says when values are not current or not available", () => {
    const live: LivePreview = { connected: true, states: [memory] }
    expect(liveValuesText("off", null)).toBeNull()
    expect(liveValuesText("live", live)).toMatch(/live values/i)
    expect(liveValuesText("live", null)).toBeNull()
    expect(liveValuesText("stopped", null)).toMatch(/not running/)
    expect(liveValuesText("disconnected", live)).toMatch(/not current/)
    expect(liveValuesText("inactiveProfile", null)).toMatch(/active/)
    expect(liveValuesText("unsupported", null)).toMatch(/restart/i)
    const statuses: LiveStatus[] = ["stopped", "disconnected"]
    for (const status of statuses) {
      render(<LiveValuesNote status={status} live={live} />)
      expect(screen.getByRole("status")).toHaveAttribute("data-state", status)
      cleanup()
    }
  })
})

describe("useLiveKeyStates", () => {
  it("asks for nothing without keys, without an agent or with an older one", () => {
    const off = renderHook(() => useLiveKeyStates("profile-1", [], agent()))
    expect(off.result.current).toEqual({ live: null, status: "off" })
    const stopped = renderHook(() =>
      useLiveKeyStates("profile-1", ["cpu"], agent({ running: false })),
    )
    expect(stopped.result.current.status).toBe("stopped")
    const old = renderHook(() =>
      useLiveKeyStates("profile-1", ["cpu"], agentConnection({ capabilities: ["plugins"] })),
    )
    expect(old.result.current.status).toBe("unsupported")
    expect(api.watchKeyStates).not.toHaveBeenCalled()
  })

  it("shows the agent's states and their changes, and stale ones when it goes", async () => {
    const hook = renderHook(
      ({ connection }) => useLiveKeyStates("profile-1", ["memory", "cpu", "cpu"], connection),
      { initialProps: { connection: agent() } },
    )
    expect(hook.result.current.status).toBe("connecting")
    // One watch of the page's actions, each once.
    await waitFor(() => {
      expect(watches()).toEqual([["profile-1", ["cpu", "memory"]]])
    })
    act(() => {
      emitBackend(KEY_STATES_EVENT, { watch: 1, kind: "snapshot", live: true, states: [cpu("1%")] })
    })
    expect(hook.result.current).toEqual({
      status: "live",
      live: { connected: true, states: [cpu("1%")] },
    })
    act(() => {
      emitBackend(KEY_STATES_EVENT, { watch: 1, kind: "changed", states: [cpu("23%"), memory] })
      // A replaced watch's late event is ignored.
      emitBackend(KEY_STATES_EVENT, { watch: 0, kind: "changed", states: [cpu("99%")] })
    })
    expect(hook.result.current.live?.states).toEqual([cpu("23%"), memory])
    // The agent went away: the last values stay, marked stale.
    act(() => {
      emitBackend(KEY_STATES_EVENT, { watch: 1, kind: "disconnected" })
    })
    expect(hook.result.current).toEqual({
      status: "disconnected",
      live: { connected: false, states: [cpu("23%"), memory] },
    })
    // It stopped: nothing is asked; the values stay stale.
    const before = api.watchKeyStates.mock.calls.length
    hook.rerender({ connection: agent({ running: false }) })
    expect(hook.result.current.status).toBe("stopped")
    expect(hook.result.current.live?.connected).toBe(false)
    expect(api.watchKeyStates.mock.calls.slice(before)).toEqual([["profile-1", []]])
  })

  it("takes a first answer that comes before the watch's number", async () => {
    let resolve: (n: number) => void = () => undefined
    api.watchKeyStates.mockImplementationOnce(
      () =>
        new Promise<number>((r) => {
          resolve = r
        }),
    )
    const hook = renderHook(() => useLiveKeyStates("profile-1", ["cpu"], agent()))
    await waitFor(() => {
      expect(api.watchKeyStates).toHaveBeenCalled()
    })
    act(() => {
      emitBackend(KEY_STATES_EVENT, { watch: 5, kind: "snapshot", live: true, states: [cpu("7%")] })
    })
    expect(hook.result.current.status).toBe("connecting")
    await act(async () => {
      resolve(5)
      await Promise.resolve()
    })
    expect(hook.result.current.live?.states).toEqual([cpu("7%")])
  })

  it("watches again on another page, profile or project and stops when closed", async () => {
    const hook = renderHook(
      ({ actions, connection }) => useLiveKeyStates("profile-1", actions, connection),
      { initialProps: { actions: ["cpu"], connection: agent() } },
    )
    await waitFor(() => {
      expect(watches()).toEqual([["profile-1", ["cpu"]]])
    })
    // Another page.
    hook.rerender({ actions: ["memory"], connection: agent() })
    await waitFor(() => {
      expect(watches().at(-1)).toEqual(["profile-1", ["memory"]])
    })
    expect(watches().at(-2)).toEqual(["profile-1", []])
    // The same actions in another order: no new watch.
    const count = watches().length
    hook.rerender({ actions: ["memory", "memory"], connection: agent() })
    expect(watches()).toHaveLength(count)
    // The agent's project changed.
    hook.rerender({
      actions: ["memory"],
      connection: agent({
        status: agentStatus({
          activeProject: { id: "p2", key: "CMS", name: "CMS" } as never,
        }),
      }),
    })
    await waitFor(() => {
      expect(watches()).toHaveLength(count + 2)
    })
    // Another profile active: the editor's profile has no live states.
    act(() => {
      emitBackend(KEY_STATES_EVENT, {
        watch: api.watchKeyStates.mock.calls.length,
        kind: "snapshot",
        live: false,
        states: [],
      })
    })
    expect(hook.result.current).toEqual({ status: "inactiveProfile", live: null })
    hook.unmount()
    expect(watches().at(-1)).toEqual(["profile-1", []])
  })
})

describe("usePreviewFaces", () => {
  it("draws the preview with the agent's live states", async () => {
    api.previewLayout.mockResolvedValue([])
    const layout = {
      id: "l",
      name: "Main",
      model: "elgato.streamdeck.mk2",
      homePage: "home",
      pages: [{ id: "home", name: "Home", bindings: [] }],
    }
    const live: LivePreview = { connected: true, states: [cpu("23%")] }
    const hook = renderHook(
      ({ current }) => usePreviewFaces("profile-1", layout as never, "home", "", current),
      { initialProps: { current: null as LivePreview | null } },
    )
    await waitFor(() => {
      expect(api.previewLayout).toHaveBeenCalledWith(
        "profile-1",
        layout,
        "home",
        null,
        true,
        null,
        null,
      )
    })
    hook.rerender({ current: live })
    await waitFor(() => {
      expect(api.previewLayout).toHaveBeenLastCalledWith(
        "profile-1",
        layout,
        "home",
        null,
        true,
        live,
        null,
      )
    })
  })
})
