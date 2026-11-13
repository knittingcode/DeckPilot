// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { AppSidebar } from "@/components/app-sidebar"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"

beforeAll(() => {
  installDomStubs()
})

afterEach(() => {
  cleanup()
})

describe("main navigation", () => {
  it("keeps the Layout Designer and no longer lists Device Models", async () => {
    const onNavigate = vi.fn()
    const { user } = renderWithApp(<AppSidebar current="dashboard" onNavigate={onNavigate} />)
    expect(screen.queryByText("Device Models")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Layout Designer" }))
    expect(onNavigate).toHaveBeenCalledWith("designer")
  })
})
