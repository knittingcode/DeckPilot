// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { DeviceLayout } from "@/lib/types"
import { DEVICE_MODELS } from "@/test/fixtures"

import { useLayoutEditing } from "./use-layout-editing"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }))

const saved: DeviceLayout = {
  id: "main",
  name: "Main",
  model: DEVICE_MODELS.mk2.id,
  homePage: "home",
  explicitTree: true,
  pages: [
    { id: "home", name: "Home", bindings: [] },
    { id: "tools", name: "Tools", parent: "home", bindings: [] },
  ],
}

describe("revert and undo keep the nearest page", () => {
  it("shows the folder a page that is gone was in, as the device does", () => {
    const { result } = renderHook(() => useLayoutEditing(DEVICE_MODELS.mk2, saved))
    act(() => {
      result.current.commands.openAt("tools", ["home"])
    })
    act(() => {
      result.current.commands.addPage("Sub", "tools")
    })
    const sub = result.current.pageId
    expect(sub).not.toBe("tools")
    act(() => {
      result.current.commands.openAt(sub, ["home", "tools"])
    })
    expect(result.current.trail).toEqual(["home", "tools"])
    // Revert: Sub is gone; Tools, which it was reached through, is shown.
    act(() => {
      result.current.commands.revert()
    })
    expect(result.current.pageId).toBe("tools")
    expect(result.current.trail).toEqual(["home"])
    // Undoing the revert brings Sub back, where the editor was.
    act(() => {
      result.current.commands.undo()
    })
    expect(result.current.pageId).toBe(sub)
    expect(result.current.trail).toEqual(["home", "tools"])
  })
})
