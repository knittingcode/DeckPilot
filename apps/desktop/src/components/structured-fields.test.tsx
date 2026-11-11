// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { ListField, PairsField } from "./structured-fields"

vi.mock("@/lib/api", () => ({ api: { listCredentials: () => Promise.resolve([]) } }))

function List({ initial, onValue }: { initial: string; onValue: (value: string) => void }) {
  const [value, setValue] = useState(initial)
  return (
    <ListField
      id="args"
      value={value}
      itemLabel="Argument"
      maxLength={20}
      onChange={(next) => {
        setValue(next)
        onValue(next)
      }}
    />
  )
}

describe("structured fields", () => {
  afterEach(cleanup)

  it("edits arguments as a list and shows the exact vector", async () => {
    const user = userEvent.setup()
    const values: string[] = []
    render(<List initial={"--name\nJohn Smith"} onValue={(v) => values.push(v)} />)
    expect(screen.getByText('["--name","John Smith"]')).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Add argument" }))
    await user.type(screen.getByLabelText("Argument 3"), "a b")
    expect(values.at(-1)).toBe("--name\nJohn Smith\na b")
    // Reorder and remove.
    await user.click(screen.getByRole("button", { name: "Move argument 3 up" }))
    expect(values.at(-1)).toBe("--name\na b\nJohn Smith")
    await user.click(screen.getByRole("button", { name: "Remove argument 1" }))
    expect(values.at(-1)).toBe("a b\nJohn Smith")
    expect(screen.getByText('["a b","John Smith"]')).toBeInTheDocument()
  })

  it("edits headers as rows and warns about secrets and duplicates", async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(
      <PairsField
        id="headers"
        value="Accept: application/json"
        separator=":"
        nameLabel="Header"
        caseInsensitive
        noSecrets
        onChange={onChange}
      />,
    )
    expect(screen.getByLabelText("Header 1")).toHaveValue("Accept")
    await user.click(screen.getByRole("button", { name: "Add header" }))
    await user.type(screen.getByLabelText("Header 2"), "Authorization")
    expect(screen.getByText(/holds a secret/)).toBeInTheDocument()
    await user.clear(screen.getByLabelText("Header 2"))
    await user.type(screen.getByLabelText("Header 2"), "accept")
    expect(screen.getByText(/appears more than once/)).toBeInTheDocument()
    await user.type(screen.getByLabelText("Value 2"), "text/plain")
    expect(onChange).toHaveBeenLastCalledWith("Accept: application/json\naccept: text/plain")
  })
})
