// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it } from "vitest"

import folderBackIcon from "../../../../crates/deckpilot-render/assets/icons/folder-back.svg?url"

import { FOLDER_BACK_SYMBOL, FaceTile } from "./key-face"

afterEach(cleanup)

describe("the fallback face of the folder's system Back", () => {
  it("draws the core-owned icon file, white on black, without text", () => {
    const { container } = render(
      <FaceTile face={{ style: "back", title: "Back", symbol: FOLDER_BACK_SYMBOL } as never} />,
    )
    const tile = container.querySelector("[data-slot=key-face]")
    const icon = container.querySelector<HTMLElement>("[data-slot=folder-back-icon]")
    expect(icon).not.toBeNull()
    expect(icon?.style.maskImage).toBe(`url("${folderBackIcon}")`)
    expect(icon).toHaveClass("bg-white")
    expect(tile).toHaveClass("bg-black")
    expect(tile).not.toHaveTextContent("Back")
    // No drawn glyph of its own.
    expect(container.querySelector("svg")).toBeNull()
  })

  it("leaves other Back keys as they were", () => {
    const { container } = render(<FaceTile face={{ style: "back", title: "Back" } as never} />)
    expect(container.querySelector("[data-slot=folder-back-icon]")).toBeNull()
    expect(container.querySelector("svg")).not.toBeNull()
    expect(container).toHaveTextContent("Back")
  })
})
