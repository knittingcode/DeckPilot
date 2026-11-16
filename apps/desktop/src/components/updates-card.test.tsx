// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { UpdatesCard } from "@/components/updates-card"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { type UpdateStateName, updateState } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
})
afterEach(cleanup)

function renderCard(name: UpdateStateName, overrides: Parameters<typeof updateState>[1] = {}) {
  return renderWithApp(<UpdatesCard />, { updates: updateState(name, overrides) })
}

/** What a build of each channel may offer (docs/distribution/README.md, section 9). */
interface Expected {
  check: "enabled" | "disabled" | "absent"
  selfUpdate: boolean
  preferences: boolean
  command: string | null
  deliveredBy: string | null
  installedFrom: string
}

const CHANNELS: [UpdateStateName, Expected][] = [
  [
    "github",
    {
      check: "enabled",
      selfUpdate: true,
      preferences: true,
      command: null,
      deliveredBy: null,
      installedFrom: "GitHub Releases (.dmg / .app)",
    },
  ],
  [
    "apt",
    {
      check: "absent",
      selfUpdate: false,
      preferences: false,
      command: "sudo apt update && sudo apt upgrade",
      deliveredBy: "Updates are delivered by APT",
      installedFrom: "APT repository",
    },
  ],
  [
    "homebrew",
    {
      check: "absent",
      selfUpdate: false,
      preferences: false,
      command: "brew upgrade --cask deckpilot",
      deliveredBy: "Updates are delivered by Homebrew",
      installedFrom: "Homebrew",
    },
  ],
  [
    "winget",
    {
      check: "absent",
      selfUpdate: false,
      preferences: false,
      command: "winget upgrade KnittingCode.DeckPilot",
      deliveredBy: "Updates are delivered by WinGet",
      installedFrom: "WinGet",
    },
  ],
  [
    "flathub",
    {
      check: "absent",
      selfUpdate: false,
      preferences: false,
      command: null,
      deliveredBy: "Updates are delivered by Flathub",
      installedFrom: "Flathub",
    },
  ],
  [
    "macAppStore",
    {
      check: "absent",
      selfUpdate: false,
      preferences: false,
      command: null,
      deliveredBy: "Updates are delivered by the Mac App Store",
      installedFrom: "Mac App Store",
    },
  ],
  [
    "microsoftStore",
    {
      check: "absent",
      selfUpdate: false,
      preferences: false,
      command: null,
      deliveredBy: "Updates are delivered by the Microsoft Store",
      installedFrom: "Microsoft Store",
    },
  ],
  [
    "notConfigured",
    {
      check: "disabled",
      selfUpdate: false,
      preferences: false,
      command: null,
      deliveredBy: null,
      installedFrom: "Development or self-built version (no update key)",
    },
  ],
]

describe("updates card per distribution channel", () => {
  it.each(CHANNELS)("%s renders only the controls it supports", (name, expected) => {
    // Every build is offered the same update, so only the channel decides.
    const { titlebar } = renderCard(name, {
      available: { version: "0.6.0", notes: "Release notes", date: null },
    })

    const check = titlebar.action("updates.check")
    if (expected.check === "absent") expect(check).toBeUndefined()
    else expect(check?.disabled).toBe(expected.check === "disabled")

    const download = screen.queryByRole("button", { name: /Download/ })
    const notes = screen.queryByText("Release notes")
    if (expected.selfUpdate) {
      expect(download).toBeInTheDocument()
      expect(notes).toBeInTheDocument()
    } else {
      expect(download).not.toBeInTheDocument()
      expect(notes).not.toBeInTheDocument()
      expect(screen.queryByRole("button", { name: /Install/ })).not.toBeInTheDocument()
    }

    for (const label of ["When an update is available", "Update channel", "Check every"]) {
      if (expected.preferences) expect(screen.getByText(label)).toBeInTheDocument()
      else expect(screen.queryByText(label)).not.toBeInTheDocument()
    }

    const command = screen.queryByLabelText("Update command")
    if (expected.command) {
      expect(command).toHaveTextContent(expected.command)
      expect(screen.getByRole("button", { name: /Copy/ })).toBeInTheDocument()
    } else {
      expect(command).not.toBeInTheDocument()
      expect(screen.queryByRole("button", { name: /Copy/ })).not.toBeInTheDocument()
    }

    if (expected.deliveredBy) expect(screen.getByText(expected.deliveredBy)).toBeInTheDocument()
    else expect(screen.queryByText(/Updates are delivered by/)).not.toBeInTheDocument()

    expect(screen.getByText(expected.installedFrom)).toBeInTheDocument()
    expect(screen.getByText("0.6.0-alpha.3")).toBeInTheDocument()
    expect(screen.getByText("Alpha (pre-release)")).toBeInTheDocument()
    // Never the GitHub wording outside GitHub builds.
    if (name !== "github") {
      expect(screen.queryByText(/come from GitHub Releases/)).not.toBeInTheDocument()
    }
  })

  it.each<UpdateStateName>(["flathub", "macAppStore", "microsoftStore"])(
    "a %s build never checks, downloads or installs",
    async (name) => {
      // Even with a downloaded update in its state.
      const { user, titlebar } = renderCard(name, {
        phase: "ready",
        available: { version: "0.6.0", notes: null, date: null },
      })
      expect(titlebar.getSnapshot().actions ?? []).toHaveLength(0)
      for (const button of screen.queryAllByRole("button")) await user.click(button)
      expect(api.installUpdate).not.toHaveBeenCalled()
      expect(api.downloadUpdate).not.toHaveBeenCalled()
      expect(api.checkForUpdates).not.toHaveBeenCalled()
      expect(api.openWindow).not.toHaveBeenCalled()
    },
  )

  it("explains how a store build is updated", () => {
    renderCard("flathub")
    expect(screen.getByText("flatpak update")).toBeInTheDocument()
    cleanup()
    renderCard("macAppStore")
    expect(screen.getByText("Updates are installed by the App Store.")).toBeInTheDocument()
    expect(screen.getByText(/quit and reopen DeckPilot/)).toBeInTheDocument()
  })

  it("copies the package manager's command", async () => {
    const { user } = renderCard("apt")
    await user.click(screen.getByRole("button", { name: /Copy/ }))
    expect(await navigator.clipboard.readText()).toBe("sudo apt update && sudo apt upgrade")
    expect(api.installUpdate).not.toHaveBeenCalled()
  })

  it("shows a GitHub build's progress, restart requirement and install", async () => {
    renderCard("githubDownloading")
    expect(screen.getByText("25%")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /Download/ })).toBeDisabled()
    cleanup()

    api.installUpdate.mockResolvedValue(null)
    api.updateState.mockResolvedValue(updateState("githubReady"))
    api.openWindow.mockResolvedValue(null)
    const { user } = renderCard("githubReady")
    expect(screen.getByText(/restarts DeckPilot with the new agent/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: /Install and restart/ }))
    expect(api.installUpdate).toHaveBeenCalled()
  })

  it.each<[UpdateStateName, string]>([
    ["githubUntrustedUrl", "Update refused: unexpected download address"],
    ["githubOtherFormat", "Update refused: different package format"],
  ])("words the refusal of %s", (name, title) => {
    renderCard(name)
    expect(screen.getByText(title)).toBeInTheDocument()
    expect(screen.getByText(/nothing was downloaded/)).toBeInTheDocument()
    expect(screen.getByText(/^Details:/)).toBeInTheDocument()
  })

  it.each<[UpdateStateName, RegExp]>([
    ["unknownFormat", /cannot tell which package this copy was installed from/],
    ["formatMismatch", /made as a \.deb package but is installed as AppImage/],
    ["formatNotUpdatable", /cannot install MSI packages on this platform/],
    ["notConfigured", /no update signing key/],
  ])("explains why %s has no updates", (name, text) => {
    const { titlebar } = renderCard(name)
    expect(screen.getByText("Updates are not available in this build")).toBeInTheDocument()
    expect(screen.getByText(text)).toBeInTheDocument()
    expect(titlebar.action("updates.check")?.disabled).toBe(true)
  })

  it("shows the outcome of recovering an interrupted update in any channel", () => {
    renderCard("apt", {
      error:
        "the update from 0.6.0 to 0.7.0 did not complete; version 0.6.0 is still installed and its agent runs again. Install the update again",
    })
    expect(screen.getByText("The last update did not complete")).toBeInTheDocument()
    expect(screen.getByText(/version 0\.6\.0 is still installed/)).toBeInTheDocument()
  })

  it("labels a development override clearly", () => {
    renderCard("developmentOverride")
    expect(screen.getByText("Development override")).toBeInTheDocument()
    expect(screen.getByText(/behaves as if installed from APT repository/)).toBeInTheDocument()
    cleanup()
    renderCard("apt")
    expect(screen.queryByText("Development override")).not.toBeInTheDocument()
  })
})
