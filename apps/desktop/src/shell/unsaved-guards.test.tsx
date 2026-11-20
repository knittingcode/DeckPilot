// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { DialogsContext } from "@/components/dialogs/use-dialogs"
import type { Dialogs } from "@/components/dialogs/dialog-service"
import { emitBackend, resetBackend } from "@/test/tauri"

import { QUIT_REQUESTED_EVENT, leavesEditor, useUnsavedGuards } from "./unsaved-guards"

const invoke = vi.hoisted(() =>
  vi.fn<(command: string, args?: unknown) => Promise<null>>(() => Promise.resolve(null)),
)
vi.mock("@tauri-apps/api/core", () => ({ invoke }))
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ onCloseRequested: () => Promise.resolve(() => undefined) }),
}))
vi.mock("sonner", () => ({ toast: { warning: vi.fn() } }))

const designer = { mode: "workspace", page: "designer" }

function setup(dirty: boolean, discard: boolean) {
  const destructive = vi.fn(() => Promise.resolve(discard))
  const dialogs = { destructive } as unknown as Dialogs
  const navigate = vi.fn()
  const setDirty = vi.fn()
  const dirtyRef = { current: dirty }
  const wrapper = ({ children }: { children: ReactNode }) => (
    <DialogsContext value={dialogs}>{children}</DialogsContext>
  )
  const hook = renderHook(() => useUnsavedGuards(dirtyRef, designer, navigate, setDirty), {
    wrapper,
  })
  return { hook, destructive, navigate, setDirty }
}

beforeEach(() => {
  resetBackend()
  invoke.mockClear()
})
afterEach(() => {
  cleanup()
})

describe("unsaved edits are never discarded without asking", () => {
  it("knows when a page change leaves the editor", () => {
    expect(leavesEditor(designer, "workflows")).toBe(true)
    // The settings keep the editor mounted under them.
    expect(leavesEditor(designer, "settings:security")).toBe(false)
    expect(leavesEditor(designer, "designer")).toBe(false)
    expect(leavesEditor({ mode: "workspace", page: "workflows" }, "dashboard")).toBe(false)
  })

  it("leaves at once without unsaved edits", () => {
    const { hook, destructive, navigate } = setup(false, false)
    hook.result.current("workflows")
    expect(navigate).toHaveBeenCalledWith("workflows")
    expect(destructive).not.toHaveBeenCalled()
  })

  it("asks before leaving the editor, and stays when cancelled", async () => {
    const stay = setup(true, false)
    stay.hook.result.current("workflows")
    await waitFor(() => {
      expect(stay.destructive).toHaveBeenCalled()
    })
    expect(stay.navigate).not.toHaveBeenCalled()
    cleanup()
    const leave = setup(true, true)
    leave.hook.result.current("workflows")
    await waitFor(() => {
      expect(leave.navigate).toHaveBeenCalledWith("workflows")
    })
    expect(leave.setDirty).toHaveBeenCalledWith(false)
  })

  it("asks before quitting, and quits only when the edits are discarded", async () => {
    const stay = setup(true, false)
    await waitFor(() => {
      expect(emitBackend(QUIT_REQUESTED_EVENT, null)).toBe(1)
    })
    await waitFor(() => {
      expect(stay.destructive).toHaveBeenCalled()
    })
    expect(invoke).not.toHaveBeenCalledWith("quit_app")
    cleanup()
    resetBackend()
    setup(true, true)
    await waitFor(() => {
      expect(emitBackend(QUIT_REQUESTED_EVENT, null)).toBe(1)
    })
    await act(async () => {
      await Promise.resolve()
    })
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("quit_app")
    })
  })
})
