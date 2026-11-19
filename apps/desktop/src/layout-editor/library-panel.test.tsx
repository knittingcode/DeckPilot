// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { DndContext } from "@dnd-kit/core"
import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it } from "vitest"

import { ACTION_TYPES } from "@/test/fixtures"

import type { CatalogGroup } from "./catalog"
import { LibraryPanel } from "./library-panel"
import type { TargetGroup } from "./targets"

const groups: TargetGroup[] = [
  {
    label: "Actions",
    options: [
      {
        value: "action:mail",
        label: "Mail",
        target: { type: "action", action: "mail" },
        kind: "action",
        icon: "mail",
      },
      {
        value: "action:mute",
        label: "Mute mic",
        target: { type: "action", action: "mute" },
        kind: "action",
        incompatible: "It needs a key with a display.",
      },
      {
        value: "action:spotify",
        label: "Spotify",
        target: { type: "action", action: "spotify" },
        kind: "action",
        unavailable: "Its plugin is disabled.",
      },
    ],
  },
  {
    label: "Navigation",
    options: [{ value: "back", label: "Back", target: { type: "back" }, kind: "navigation" }],
  },
]
const catalog: CatalogGroup[] = [
  { label: "Docker", entries: [{ type: ACTION_TYPES.dockerStatus, module: "Docker" }] },
]

function renderPanel(variant: "plain" | "card" = "plain") {
  const user = userEvent.setup()
  render(
    <DndContext>
      <LibraryPanel
        groups={groups}
        catalog={catalog}
        multiAction
        variant={variant}
        compatibility={{ control: "Key 1", catalog: () => null }}
      />
    </DndContext>,
  )
  return user
}

beforeEach(() => {
  localStorage.clear()
})
afterEach(cleanup)

describe("the library's rows", () => {
  it("are compact rows with one icon size, not boxes", () => {
    renderPanel()
    const rows = [
      "Drag Mail onto a control",
      "Drag Back onto a control",
      "Drag New folder onto an empty control",
      "Drag Multi Action onto a control",
      "Drag Docker status onto a control",
    ].map((name) => screen.getByRole("button", { name }))
    for (const row of rows) {
      expect(row).toHaveClass("h-8")
      expect(row.className).not.toMatch(/\b(border|border-dashed|bg-card)\b/)
      expect(row.querySelector("[data-slot=library-icon] svg")).toBeInTheDocument()
    }
    // Headings: small, muted, uppercase, with the count.
    const heading = within(screen.getByRole("region", { name: "Actions" })).getByRole("button", {
      name: /Actions/,
    })
    expect(heading).toHaveClass("uppercase", "text-muted-foreground")
    expect(heading).toHaveTextContent("3")
  })

  it("says why an entry cannot be placed, and filters", async () => {
    const user = renderPanel()
    const mute = screen.getByRole("listitem", { name: /^Mute mic: not for the selected control/ })
    expect(mute).toHaveAttribute("aria-disabled", "true")
    expect(mute).toHaveTextContent("It needs a key with a display.")
    expect(mute.className).not.toMatch(/border-dashed/)
    expect(screen.getByRole("listitem", { name: /^Spotify: unavailable/ })).toHaveTextContent(
      "Its plugin is disabled.",
    )

    await user.click(screen.getByRole("switch", { name: "Compatible only" }))
    expect(screen.queryByRole("listitem", { name: /^Mute mic/ })).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Drag Mail onto a control" })).toBeInTheDocument()

    await user.type(screen.getByRole("searchbox", { name: "Search the library" }), "docker")
    expect(screen.queryByRole("button", { name: "Drag Mail onto a control" })).toBeNull()
    expect(screen.getByRole("button", { name: "Drag Docker status onto a control" })).toBeVisible()
  })

  it("still comes as a card of its own", () => {
    renderPanel("card")
    expect(screen.getByText("Library", { selector: "[data-slot=card-title]" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Drag Mail onto a control" })).toHaveClass("h-8")
  })
})
