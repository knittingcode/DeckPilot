// @vitest-environment jsdom
// Optional permissions (plugin API 1.9, `requiresPermissions`): the
// consent dialog lists them with their own check that never blocks the
// confirm; a high-tier one starts off, a lower one on; the consent is
// exactly what is checked. The permission list names the actions that
// need one.
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { installDomStubs } from "@/test/dom"

import { PermissionList } from "./components"
import { actionNamesOf, grantOfInfo, optionalOnByDefault } from "./permissions"
import { PermissionReviewDialog, type ReviewRequest } from "./review-dialog"
import type { GrantInfo } from "./types"

beforeAll(installDomStubs)
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

const NAMES = actionNamesOf([
  { action: "status", name: "Docker status" },
  { action: "compose-up", name: "Compose up" },
  { action: "compose-down", name: "Compose down" },
])

function info(extra: Partial<GrantInfo> & { key: string }): GrantInfo {
  return {
    kind: extra.key.split(":")[0] ?? extra.key,
    tier: "high",
    status: "notGranted",
    scope: null,
    justification: "Runs your own docker.",
    ...extra,
  }
}

const READ = info({ key: "tools:docker", scope: { type: "tool", subcommands: { ps: [] } } })
const MODIFY = info({
  key: "tools:docker:modify",
  scope: { type: "toolModify" },
  optional: true,
  actions: ["compose-up", "compose-down"],
})
const STATE = info({
  key: "device:state",
  tier: "low",
  scope: { type: "deviceState" },
  optional: true,
  actions: ["compose-up"],
})

function request(confirm: ReviewRequest["confirm"], items = [READ, MODIFY]): ReviewRequest {
  return {
    mode: "enable",
    pluginId: "deckpilot.docker",
    name: "Docker",
    publisher: "DeckPilot",
    version: "0.1.0",
    items: items.map((i) => ({ grant: grantOfInfo(i, NAMES), state: "new" as const })),
    confirm,
  }
}

describe("optional permissions", () => {
  it("names the actions that need one and starts a high-tier one off", () => {
    const modify = grantOfInfo(MODIFY, NAMES)
    expect(modify.optionalFor).toEqual(["Compose up", "Compose down"])
    expect(optionalOnByDefault(modify)).toBe(false)
    expect(optionalOnByDefault(grantOfInfo(STATE, NAMES))).toBe(true)
    expect(grantOfInfo(READ, NAMES).optionalFor).toBeUndefined()

    render(<PermissionList label="Permissions" grants={[grantOfInfo(MODIFY, NAMES)]} />)
    expect(
      screen.getByText(/only these actions need it: Compose up, Compose down/),
    ).toBeInTheDocument()
  })

  it("enables with the required permissions only, the optional one off", async () => {
    const user = userEvent.setup()
    const confirm = vi.fn(() => Promise.resolve())
    render(<PermissionReviewDialog request={request(confirm)} onClose={() => {}} />)
    const optional = document.getElementById("consent-tools:docker:modify")
    if (!optional) throw new Error("no optional check")
    expect(optional).not.toBeChecked()
    // The optional one does not count: one required permission is missing.
    const enable = screen.getByRole("button", { name: "Enable" })
    expect(enable).toBeDisabled()
    const boxes = screen.getAllByRole("checkbox")
    const required = boxes.find((b) => b !== optional)
    if (!required) throw new Error("no required check")
    await user.click(required)
    expect(enable).toBeEnabled()
    await user.click(enable)
    expect(confirm).toHaveBeenCalledWith(["tools:docker"])
  })

  it("sends an optional permission the user checked, and a low-tier one starts on", async () => {
    const user = userEvent.setup()
    const confirm = vi.fn(() => Promise.resolve())
    render(
      <PermissionReviewDialog
        request={request(confirm, [READ, MODIFY, STATE])}
        onClose={() => {}}
      />,
    )
    expect(
      screen.getByText("Optional: allow", { selector: "label[for='consent-device:state']" }),
    ).toBeInTheDocument()
    const boxes = screen.getAllByRole("checkbox")
    expect(
      boxes.filter((b) => (b as HTMLInputElement).getAttribute("aria-checked") === "true"),
    ).toHaveLength(1)
    for (const box of boxes) {
      if (box.getAttribute("aria-checked") !== "true") await user.click(box)
    }
    await user.click(screen.getByRole("button", { name: "Enable" }))
    expect(confirm).toHaveBeenCalledWith(["tools:docker", "tools:docker:modify", "device:state"])
  })
})
