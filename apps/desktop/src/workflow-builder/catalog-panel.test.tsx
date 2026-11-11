// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { DRAG_TYPE, flowEntries } from "./catalog"
import { CatalogPanel } from "./catalog-panel"

const groups = [{ id: "flow", title: "Flow", entries: flowEntries(undefined), types: [] }]

describe("node catalog", () => {
  afterEach(cleanup)

  it("drags entries with their key and adds them on click or keyboard", () => {
    const onAdd = vi.fn()
    render(<CatalogPanel groups={groups} onAdd={onAdd} onCreateAction={vi.fn()} />)
    const delay = screen.getByRole("listitem", { name: /Delay/ })
    const setData = vi.fn()
    fireEvent.dragStart(delay, { dataTransfer: { setData, effectAllowed: "" } })
    expect(setData).toHaveBeenCalledWith(DRAG_TYPE, "flow:delay")
    // A button: Enter and Space activate it like a click.
    fireEvent.click(delay)
    expect(onAdd).toHaveBeenCalledWith(expect.objectContaining({ key: "flow:delay" }))
    expect(delay.tagName).toBe("BUTTON")
  })

  it("filters by search", () => {
    render(<CatalogPanel groups={groups} onAdd={vi.fn()} onCreateAction={vi.fn()} />)
    fireEvent.change(screen.getByLabelText("Search nodes"), { target: { value: "wait" } })
    expect(screen.getAllByRole("listitem").map((e) => e.textContent)).toEqual([
      expect.stringContaining("Join"),
      expect.stringContaining("Delay"),
    ])
    fireEvent.change(screen.getByLabelText("Search nodes"), { target: { value: "zzz" } })
    expect(screen.getByText("No matching nodes")).toBeTruthy()
  })
})
