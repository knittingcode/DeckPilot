// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { deviceModel } from "@/test/fixtures"

import { VerificationNotice } from "./layout-designer"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)

afterEach(() => {
  cleanup()
})

describe("Layout Designer verification notice", () => {
  it("is not shown where the model was physically verified", () => {
    const { container } = render(<VerificationNotice model={deviceModel()} platform="macos" />)
    expect(container).toBeEmptyDOMElement()
  })

  it("says what is known on other platforms", () => {
    render(<VerificationNotice model={deviceModel()} platform="windows" />)
    expect(screen.getByText("Automated tests only on Windows")).toBeInTheDocument()
    expect(screen.getByText("Automated tests only.")).toBeInTheDocument()
  })
})
