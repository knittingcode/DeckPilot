// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { formatCandidates, parseCandidates } from "./candidates"
import { CandidatesField } from "./candidates-field"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const actions = [
  { id: "a1", name: "Hello", actionType: "general.log" },
  { id: "dice", name: "Dice", actionType: "deck.random" },
]
const workflows = [{ id: "w1", name: "Build" }]

function Field({ initial, onText }: { initial: string; onText: (text: string) => void }) {
  const [text, setText] = useState(initial)
  return (
    <CandidatesField
      id="candidates"
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

describe("Random Action candidates", () => {
  it("reads and writes the agent's lines, weights only where they are not 1", () => {
    const text = "action:a1\nworkflow:w1 3\nsomething else"
    const lines = parseCandidates(text)
    expect(lines).toEqual([
      { target: "action:a1", weight: 1 },
      { target: "workflow:w1", weight: 3 },
      { target: "", weight: 1, raw: "something else" },
    ])
    expect(formatCandidates(lines)).toBe(text)
    expect(parseCandidates("action:a1 0")[0]?.raw).toBe("action:a1 0")
  })

  it("adds actions and workflows, sets weights and removes rows", async () => {
    const onText = vi.fn()
    const user = userEvent.setup()
    render(<Field initial="" onText={onText} />)
    expect(screen.getByText("Add what the key picks from.")).toBeInTheDocument()

    await user.click(screen.getByRole("combobox", { name: "Add a candidate" }))
    // Another Random Action is not offered: picks do not nest.
    expect(screen.queryByRole("option", { name: "Dice" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("option", { name: "Hello" }))
    await user.click(screen.getByRole("combobox", { name: "Add a candidate" }))
    await user.click(screen.getByRole("option", { name: "Build" }))
    expect(onText).toHaveBeenLastCalledWith("action:a1\nworkflow:w1")

    fireEvent.change(screen.getByRole("spinbutton", { name: "Weight of candidate 2" }), {
      target: { value: "4" },
    })
    expect(onText).toHaveBeenLastCalledWith("action:a1\nworkflow:w1 4")

    await user.click(screen.getByRole("button", { name: "Remove candidate 1" }))
    expect(onText).toHaveBeenLastCalledWith("workflow:w1 4")
  })

  it("keeps what no longer exists, shown as missing", () => {
    render(<Field initial="workflow:gone" onText={vi.fn()} />)
    expect(screen.getByRole("combobox", { name: "Candidate 1" })).toHaveTextContent(
      "Missing (deleted)",
    )
  })
})
