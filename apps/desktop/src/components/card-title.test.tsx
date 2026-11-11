// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "vitest"

import { Card, CardHeader, CardTitle } from "@/components/ui/card"

afterEach(cleanup)

describe("CardTitle", () => {
  it("is a level 3 heading by default, with the card title styles", () => {
    render(
      <Card>
        <CardHeader>
          <CardTitle>Library</CardTitle>
        </CardHeader>
      </Card>,
    )
    const title = screen.getByRole("heading", { level: 3, name: "Library" })
    expect(title).toHaveAttribute("data-slot", "card-title")
    expect(title).toHaveClass("leading-none", "font-semibold")
  })

  it("takes another heading level, or no heading where none is allowed", () => {
    render(
      <>
        <CardTitle as="h2">Status</CardTitle>
        <button type="button">
          <CardTitle as="div">12</CardTitle>
        </button>
      </>,
    )
    expect(screen.getByRole("heading", { level: 2, name: "Status" })).toBeInTheDocument()
    expect(screen.queryByRole("heading", { name: "12" })).not.toBeInTheDocument()
  })
})
