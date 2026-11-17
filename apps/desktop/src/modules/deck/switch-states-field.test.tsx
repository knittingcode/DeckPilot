// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { ParameterField } from "@/components/parameter-field"
import type { ParameterSpec } from "@/lib/types"

import { formatStates, parseStates } from "./switch-states"
import { SwitchStatesField } from "./switch-states-field"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const actions = [
  { id: "on", name: "Lamp on", actionType: "general.log" },
  { id: "dice", name: "Dice", actionType: "deck.random" },
  { id: "other", name: "Other switch", actionType: "deck.multi-switch" },
]
const workflows = [{ id: "off", name: "Lamp off" }]

function Field({ initial, onText }: { initial: string; onText: (text: string) => void }) {
  const [text, setText] = useState(initial)
  return (
    <SwitchStatesField
      id="states"
      value={text}
      actions={actions}
      workflows={workflows}
      onChange={(next) => {
        setText(next)
        onText(next)
      }}
    />
  )
}

afterEach(cleanup)

describe("Multi Action Switch states", () => {
  it("reads and writes the agent's lines and keeps what it cannot read", () => {
    const text = "action:on | On | play\nworkflow:off | Off\naction:x | | star\nnonsense"
    const lines = parseStates(text)
    expect(lines.slice(0, 3)).toEqual([
      { target: "action:on", title: "On", glyph: "play" },
      { target: "workflow:off", title: "Off", glyph: "" },
      { target: "action:x", title: "", glyph: "star" },
    ])
    expect(lines[3]?.raw).toBe("nonsense")
    expect(formatStates(lines)).toBe(text)
    // An unknown glyph is not read as one.
    expect(parseStates("action:on | x | sparkles")[0]?.raw).toBeDefined()
    // A title cannot break the format.
    expect(formatStates([{ target: "action:on", title: "a|b", glyph: "" }])).toBe("action:on | a/b")
  })

  it("adds, titles, orders and removes states", async () => {
    const onText = vi.fn()
    const user = userEvent.setup()
    render(<Field initial="" onText={onText} />)
    expect(screen.getByText(/Add at least two/)).toBeInTheDocument()

    await user.click(screen.getByRole("combobox", { name: "Add a state" }))
    // Random Actions and other switches are not offered: they do not nest.
    expect(screen.queryByRole("option", { name: "Dice" })).not.toBeInTheDocument()
    expect(screen.queryByRole("option", { name: "Other switch" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("option", { name: "Lamp on" }))
    await user.click(screen.getByRole("combobox", { name: "Add a state" }))
    await user.click(screen.getByRole("option", { name: "Lamp off" }))
    expect(onText).toHaveBeenLastCalledWith("action:on\nworkflow:off")

    fireEvent.change(screen.getByRole("textbox", { name: "Title of state 1" }), {
      target: { value: "On" },
    })
    await user.click(screen.getByRole("combobox", { name: "Glyph of state 1" }))
    await user.click(screen.getByRole("option", { name: "play" }))
    expect(onText).toHaveBeenLastCalledWith("action:on | On | play\nworkflow:off")

    await user.click(screen.getByRole("button", { name: "Move state 2 up" }))
    expect(onText).toHaveBeenLastCalledWith("workflow:off\naction:on | On | play")
    await user.click(screen.getByRole("button", { name: "Remove state 1" }))
    expect(onText).toHaveBeenLastCalledWith("action:on | On | play")
  })

  it("is the editor of the switch's states in the action form", () => {
    const spec: ParameterSpec = {
      key: "states",
      label: "States",
      kind: { type: "lines" },
      required: true,
    } as unknown as ParameterSpec
    render(
      <ParameterField
        spec={spec}
        value={{ type: "text", value: "action:gone | Old" }}
        actionType="deck.multi-switch"
        targets={{ actions, workflows }}
        onChange={vi.fn()}
      />,
    )
    expect(screen.getByRole("combobox", { name: "Runs in state 1" })).toHaveTextContent(
      "Missing (deleted)",
    )
    expect(screen.getByRole("textbox", { name: "Title of state 1" })).toHaveValue("Old")
  })
})
