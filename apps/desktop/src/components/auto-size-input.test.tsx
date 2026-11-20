// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { AutoSizeInput } from "./auto-size-input"

/**
 * jsdom has no layout: the mirror reports a width as a browser would for
 * its text, 7px per character plus its 14px of padding and border — the
 * field must take its width from the mirror, whatever it measures.
 */
function measureMirror() {
  return vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    const width = this.dataset.slot === "auto-size-mirror" ? this.textContent.length * 7 + 14 : 0
    return { width, height: 28, top: 0, left: 0, right: width, bottom: 28, x: 0, y: 0 } as DOMRect
  })
}

function Field({ initial, min, max }: { initial: string; min?: number; max?: number }) {
  const [value, setValue] = useState(initial)
  return (
    <AutoSizeInput
      aria-label="Name"
      value={value}
      minWidth={min}
      maxWidth={max}
      onChange={(event) => {
        setValue(event.target.value)
      }}
    />
  )
}

const field = () => screen.getByRole("textbox", { name: "Name" })
const width = () => Number.parseFloat(field().style.width)

let spy: ReturnType<typeof measureMirror>
beforeEach(() => {
  spy = measureMirror()
})
afterEach(() => {
  spy.mockRestore()
  cleanup()
})

describe("AutoSizeInput", () => {
  it("is as wide as its text from the first paint, with room for the caret", () => {
    render(<Field initial="Środowisko testowe żółć" />)
    // 23 characters, Polish ones included: measured, not counted bytes.
    expect(width()).toBe(23 * 7 + 14 + 2)
  })

  it("grows while typing and shrinks while deleting", async () => {
    const user = userEvent.setup()
    render(<Field initial="Strona" min={10} />)
    const start = width()
    await user.type(field(), " druga")
    expect(width()).toBeGreaterThan(start)
    await user.type(field(), "{Backspace}{Backspace}{Backspace}{Backspace}{Backspace}{Backspace}")
    expect(width()).toBe(start)
  })

  it("keeps between its minimum and maximum; beyond, the text scrolls inside", () => {
    render(<Field initial="A" min={64} />)
    expect(width()).toBe(64)
    cleanup()
    render(<Field initial={"Bardzo długa nazwa strony ".repeat(4)} max={200} />)
    expect(width()).toBe(200)
    // The value is whole: never cut, the field scrolls.
    expect(field()).toHaveValue("Bardzo długa nazwa strony ".repeat(4))
    expect(field().className).not.toMatch(/truncate|text-ellipsis/)
  })

  it("passes key events to its own handler only", () => {
    const outer = vi.fn()
    render(
      <div onKeyDown={outer}>
        <AutoSizeInput
          aria-label="Name"
          value="x"
          readOnly
          onKeyDown={(event) => {
            event.stopPropagation()
          }}
        />
      </div>,
    )
    fireEvent.keyDown(field(), { key: "Delete" })
    expect(outer).not.toHaveBeenCalled()
  })
})
