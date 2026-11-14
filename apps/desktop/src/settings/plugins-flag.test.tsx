// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { SettingsSidebar } from "@/components/settings-sidebar"
import {
  findSettingsPage,
  groupedSettings,
  searchSettings,
  settingsPages,
} from "@/settings/registry"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"

// Records whether any plugin code is loaded at all.
const loaded = vi.hoisted(() => new Set<string>())
vi.mock("@/plugins/marketplace", () => {
  loaded.add("marketplace")
  return { PluginsSettings: () => null }
})
vi.mock("@/plugins/fixtures", () => {
  loaded.add("fixtures")
  return {}
})

beforeAll(installDomStubs)
afterEach(() => {
  cleanup()
  vi.unstubAllEnvs()
})

describe("plugin marketplace flag", () => {
  it("is off unless the build sets VITE_DECKPILOT_PLUGINS=1", () => {
    expect(import.meta.env.VITE_DECKPILOT_PLUGINS).toBeUndefined()
  })

  it("leaves the marketplace out of settings, search and routes when off", () => {
    for (const value of [undefined, "", "0", "true"]) {
      if (value !== undefined) vi.stubEnv("VITE_DECKPILOT_PLUGINS", value)
      const pages = settingsPages([])
      expect(pages.map((p) => p.id)).not.toContain("plugins")
      // The navigation only opens pages the registry knows.
      expect(findSettingsPage([], "plugins")).toBeUndefined()
      expect(searchSettings(pages, "marketplace")).toEqual([])
      expect(
        groupedSettings(pages)
          .find((g) => g.id === "extensions")
          ?.pages.map((p) => p.title),
      ).toEqual(["Modules", "Permissions"])
    }
    renderWithApp(<SettingsSidebar current="modules" />)
    expect(screen.queryByRole("button", { name: "Plugins" })).not.toBeInTheDocument()
    expect(screen.queryByText(/marketplace/i)).not.toBeInTheDocument()
    expect(loaded.size).toBe(0)
  })

  it("adds Settings › Extensions › Plugins when on, without loading it before it opens", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const page = findSettingsPage([], "plugins")
    expect(page).toMatchObject({ title: "Plugins", category: "extensions" })
    expect(searchSettings(settingsPages([]), "marketplace").map((p) => p.id)).toEqual(["plugins"])
    const { user, context } = renderWithApp(<SettingsSidebar current="modules" />)
    await user.click(screen.getByRole("button", { name: "Plugins" }))
    expect(context.openSettings).toHaveBeenCalledWith("plugins")
    // The page's code loads only when the page renders.
    expect(loaded.size).toBe(0)
  })
})
