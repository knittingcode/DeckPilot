import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

const invoke = vi.fn<(...args: unknown[]) => Promise<void>>(() => Promise.resolve())
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
const inTauri = vi.fn(() => true)
vi.mock("@/lib/window-sync", async (original) => ({
  ...(await original<typeof import("@/lib/window-sync")>()),
  inTauri: () => inTauri(),
}))

import { applyLanguage } from "@/i18n"

import { SET_AGENT_LOCALE, startAgentLocaleSync } from "./agent-locale"

describe("startAgentLocaleSync", () => {
  beforeEach(async () => {
    invoke.mockClear()
    inTauri.mockReturnValue(true)
    await applyLanguage("en")
  })
  afterEach(async () => {
    await applyLanguage("en")
  })

  it("sends the language at once and after every change, as a tag only", async () => {
    const stop = startAgentLocaleSync()
    expect(invoke).toHaveBeenLastCalledWith(SET_AGENT_LOCALE, { tag: "en" })
    await applyLanguage("pl")
    expect(invoke).toHaveBeenLastCalledWith(SET_AGENT_LOCALE, { tag: "pl" })
    stop()
    invoke.mockClear()
    await applyLanguage("en")
    expect(invoke).not.toHaveBeenCalled()
  })

  it("does nothing outside the app", () => {
    inTauri.mockReturnValue(false)
    const stop = startAgentLocaleSync()
    expect(invoke).not.toHaveBeenCalled()
    stop()
  })
})
