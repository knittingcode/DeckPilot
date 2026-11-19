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
  it("leads to the device editor and the other workspace pages", async () => {
    const onNavigate = vi.fn()
    const { user } = renderWithApp(<AppSidebar current="dashboard" onNavigate={onNavigate} />)
    expect(screen.getByRole("button", { name: "Dashboard" })).toHaveAttribute(
      "aria-current",
      "page",
    )
    await user.click(screen.getByRole("button", { name: "Device Editor" }))
    expect(onNavigate).toHaveBeenCalledWith("designer")
    for (const name of ["Layouts", "Action Library", "Workflows"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument()
    }
  })

  it("leaves out what moved to the toolbar and the settings window", () => {
    renderWithApp(<AppSidebar current="designer" onNavigate={vi.fn()} />)
    for (const name of ["Device Models", "Profiles", "Settings", "Layout Designer"]) {
      expect(screen.queryByRole("button", { name })).not.toBeInTheDocument()
    }
    expect(screen.queryByRole("button", { name: /Choose the profile/ })).not.toBeInTheDocument()
  })
})
