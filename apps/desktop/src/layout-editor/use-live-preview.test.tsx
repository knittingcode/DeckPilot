// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { StrictMode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type {
  AgentConnection,
  DeviceInfo,
  DeviceLayout,
  DeviceNavigation,
  LiveSyncUpdate,
} from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { agentConnection, deviceInfo } from "@/test/fixtures"
import { emitBackend, resetBackend } from "@/test/tauri"

import {
  LIVE_DEBOUNCE_MS,
  LIVE_MAX_WAIT_MS,
  LIVE_PREVIEW_EVENT,
  type LivePreviewTarget,
  liveEligibility,
  useLivePreview,
} from "./use-live-preview"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

function agent(change: Partial<AgentConnection> = {}): AgentConnection {
  return agentConnection({ capabilities: ["livePreview"], protocol: 16, ...change })
}

const device: DeviceInfo = deviceInfo({
  layout: { type: "compatible", layout: "main", name: "Main" },
  page: "Home",
  pageId: "home",
})

function layout(name = "Main"): DeviceLayout {
  return {
    id: "main",
    name,
    model: "elgato.streamdeck.mk2",
    homePage: "home",
    pages: [
      { id: "home", name: "Home", bindings: [] },
      { id: "tools", name: "Tools", parent: "home", bindings: [] },
    ],
  }
}

function navigation(stack: string[], seq: number, change: Partial<DeviceNavigation> = {}) {
  return {
    profile: "profile-1",
    layout: "main",
    stack,
    connected: true,
    inScope: true,
    seq,
    ...change,
  }
}

function emit(update: LiveSyncUpdate, session = 1) {
  act(() => {
    emitBackend(LIVE_PREVIEW_EVENT, { session, ...update })
  })
}

type Props = Partial<LivePreviewTarget>

function setup(initial: Props = {}) {
  const onDevice = vi.fn()
  const base: LivePreviewTarget = {
    agent: agent(),
    device,
    profileId: "profile-1",
    layoutId: "main",
    pageId: "home",
    trail: [],
    draft: layout(),
    onDevice,
  }
  const hook = renderHook((props: Props) => useLivePreview({ ...base, ...initial, ...props }), {
    initialProps: {},
  })
  return { hook, onDevice }
}

function updates() {
  return api.livePreviewUpdate.mock.calls.map(([, update]) => update)
}

beforeEach(() => {
  resetApi()
  resetBackend()
  let number = 0
  api.livePreviewBegin.mockImplementation(() => Promise.resolve(++number))
  api.livePreviewUpdate.mockResolvedValue(true)
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

describe("liveEligibility", () => {
  it("needs a running agent of protocol 16, a connected device and the active profile", () => {
    expect(liveEligibility(agent(), device, "profile-1")).toBeNull()
    expect(liveEligibility(agent({ running: false }), device, "profile-1")).toBe("stopped")
    expect(liveEligibility(agent(), null, "profile-1")).toBe("off")
    expect(liveEligibility(agent(), { ...device, state: "disconnected" }, "profile-1")).toBe("off")
    expect(
      liveEligibility(agentConnection({ capabilities: ["keyStates"] }), device, "profile-1"),
    ).toBe("unsupported")
    expect(liveEligibility(agent(), device, "profile-2")).toBe("otherProfile")
  })
})

describe("useLivePreview", () => {
  it("asks for nothing when it cannot run, and says why", () => {
    const { hook } = setup({ agent: agentConnection({ capabilities: [], protocol: 15 }) })
    expect(hook.result.current.status).toBe("unsupported")
    expect(hook.result.current.live).toBe(false)
    expect(api.livePreviewBegin).not.toHaveBeenCalled()
  })

  it("begins, sends the draft once, follows the device and ends", async () => {
    const { hook, onDevice } = setup()
    await waitFor(() => {
      expect(api.livePreviewBegin).toHaveBeenCalledWith(device.id, "profile-1", "main")
    })
    expect(hook.result.current.status).toBe("connecting")
    emit({ kind: "started", navigation: navigation(["home"], 1), resumed: false })
    expect(hook.result.current.status).toBe("live")
    expect(updates()).toEqual([{ revision: 1, layout: layout() }])
    expect(onDevice).toHaveBeenCalledWith(navigation(["home"], 1))
    // The device went into Tools by itself: the editor is told.
    emit({ kind: "navigated", navigation: navigation(["home", "tools"], 2), own: false })
    expect(onDevice).toHaveBeenLastCalledWith(navigation(["home", "tools"], 2))
    // An older state that arrives late is ignored.
    emit({ kind: "navigated", navigation: navigation(["home"], 1), own: false })
    expect(onDevice).toHaveBeenCalledTimes(2)
    // Another session's event too.
    emit({ kind: "navigated", navigation: navigation(["home"], 9), own: false }, 7)
    expect(onDevice).toHaveBeenCalledTimes(2)
    hook.unmount()
    expect(api.livePreviewEnd).toHaveBeenCalledWith(1)
  })

  it("begins one session per editor and device, also in React's strict mode", async () => {
    const onDevice = vi.fn()
    const hook = renderHook(
      () =>
        useLivePreview({
          agent: agent(),
          device,
          profileId: "profile-1",
          layoutId: "main",
          pageId: "home",
          trail: [],
          draft: layout(),
          onDevice,
        }),
      { wrapper: StrictMode },
    )
    await waitFor(() => {
      expect(api.livePreviewBegin).toHaveBeenCalled()
    })
    // Selections and edits re-render; the session stays.
    hook.rerender()
    hook.rerender()
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(api.livePreviewBegin).toHaveBeenCalledTimes(1)
    expect(api.livePreviewEnd).not.toHaveBeenCalled()
  })

  it("debounces edits, sends them at least every max wait and the latest one", async () => {
    const { hook } = setup()
    await waitFor(() => {
      expect(api.livePreviewBegin).toHaveBeenCalled()
    })
    emit({ kind: "started", navigation: navigation(["home"], 1), resumed: false })
    vi.useFakeTimers()
    for (let i = 0; i < 10; i++) {
      hook.rerender({ draft: layout(`Edit ${i}`) })
      act(() => {
        vi.advanceTimersByTime(LIVE_DEBOUNCE_MS / 2)
      })
    }
    // 10 edits 20 ms apart: sent at least every 120 ms, not 10 times.
    const sent = updates().slice(1)
    expect(sent.length).toBeGreaterThanOrEqual(1)
    expect(sent.length).toBeLessThanOrEqual(
      Math.ceil((10 * LIVE_DEBOUNCE_MS) / 2 / LIVE_MAX_WAIT_MS),
    )
    act(() => {
      vi.advanceTimersByTime(LIVE_DEBOUNCE_MS)
    })
    const last = updates().at(-1)
    expect(last?.layout?.name).toBe("Edit 9")
    // Revisions only grow.
    const revisions = updates().map((u) => u.revision ?? 0)
    expect(revisions).toEqual([...revisions].sort((a, b) => a - b))
  })

  it("sends the editor's navigation after a pending draft, without echoing the device", async () => {
    const { hook, onDevice } = setup()
    await waitFor(() => {
      expect(api.livePreviewBegin).toHaveBeenCalled()
    })
    emit({ kind: "started", navigation: navigation(["home"], 1), resumed: false })
    vi.useFakeTimers()
    // A new page and the editor going there, in one render.
    const draft = layout("With a new page")
    draft.pages.push({ id: "fresh", name: "Fresh", parent: "home", bindings: [] })
    hook.rerender({ draft, pageId: "fresh", trail: ["home"] })
    const sent = updates()
    expect(sent.at(-2)?.layout?.name).toBe("With a new page")
    expect(sent.at(-1)).toEqual({
      navigation: { type: "show", layout: "main", stack: ["home", "fresh"] },
    })
    // Device events before the answer are older: ignored.
    emit({ kind: "navigated", navigation: navigation(["home"], 2), own: false })
    expect(onDevice).toHaveBeenCalledTimes(1)
    emit({ kind: "navigated", navigation: navigation(["home", "fresh"], 3), own: true })
    emit({ kind: "navigated", navigation: navigation(["home"], 4), own: false })
    expect(onDevice).toHaveBeenCalledTimes(2)
    // Following the device does not send it back.
    const before = updates().length
    hook.rerender({ draft, pageId: "home", trail: [] })
    expect(updates().length).toBe(before)
  })

  it("moves the device to the layout the editor chose", async () => {
    setup({ layoutId: "main" })
    await waitFor(() => {
      expect(api.livePreviewBegin).toHaveBeenCalled()
    })
    emit({
      kind: "started",
      navigation: navigation(["other-home"], 1, { layout: "other", inScope: false }),
      resumed: false,
    })
    expect(updates().at(-1)).toEqual({
      navigation: { type: "show", layout: "main", stack: ["home"] },
    })
  })

  it("reports a suspended, disconnected, refused or old agent session", async () => {
    const { hook } = setup()
    await waitFor(() => {
      expect(api.livePreviewBegin).toHaveBeenCalled()
    })
    emit({ kind: "started", navigation: navigation(["home"], 1), resumed: false })
    emit({
      kind: "navigated",
      navigation: navigation(["x"], 2, { layout: "other", inScope: false }),
      own: false,
    })
    expect(hook.result.current.status).toBe("suspended")
    emit({ kind: "disconnected" })
    expect(hook.result.current.status).toBe("disconnected")
    emit({ kind: "refused", reason: "livePreview.tooLarge", message: "too large" })
    expect(hook.result.current).toMatchObject({ status: "refused", problem: "too large" })
    // Resumed after the agent came back: the backend resends; nothing here.
    const before = updates().length
    emit({ kind: "started", navigation: navigation(["home"], 1), resumed: true })
    expect(hook.result.current.status).toBe("live")
    expect(updates().length).toBe(before)
    emit({ kind: "unsupported" })
    expect(hook.result.current.status).toBe("unsupported")
  })

  it("flushes a pending draft at once (before saving)", async () => {
    const { hook } = setup()
    await waitFor(() => {
      expect(api.livePreviewBegin).toHaveBeenCalled()
    })
    emit({ kind: "started", navigation: navigation(["home"], 1), resumed: false })
    vi.useFakeTimers()
    hook.rerender({ draft: layout("To save") })
    const before = updates().length
    act(() => {
      hook.result.current.flush()
    })
    expect(updates().length).toBe(before + 1)
    expect(updates().at(-1)?.layout?.name).toBe("To save")
  })
})
