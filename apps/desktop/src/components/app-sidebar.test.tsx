// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { AppSidebar } from "@/components/app-sidebar"
import { pluginModules } from "@/lib/plugin-actions"
import { projectsPlugin, workflowsPlugin } from "@/test/first-party"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"

beforeAll(() => {
  installDomStubs()
})

afterEach(() => {
  cleanup()
  vi.unstubAllEnvs()
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

describe("main navigation in a build with plugins", () => {
  it("shows Workflows and Projects only while their plugins are enabled", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const onNavigate = vi.fn()
    const { user, rerender, context } = renderWithApp(
      <AppSidebar current="designer" onNavigate={onNavigate} />,
      { modules: [], pluginModules: [] },
    )
    expect(screen.queryByRole("button", { name: "Workflows" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Projects" })).not.toBeInTheDocument()

    rerender(<AppSidebar current="designer" onNavigate={onNavigate} />, {
      ...context,
      pluginModules: pluginModules([workflowsPlugin(), projectsPlugin()]),
    })
    await user.click(screen.getByRole("button", { name: "Workflows" }))
    expect(onNavigate).toHaveBeenCalledWith("workflows")
    await user.click(screen.getByRole("button", { name: "Projects" }))
    expect(onNavigate).toHaveBeenCalledWith("projects/projects")

    rerender(<AppSidebar current="designer" onNavigate={onNavigate} />, {
      ...context,
      pluginModules: pluginModules([workflowsPlugin("disabled"), projectsPlugin("disabled")]),
    })
    expect(screen.queryByRole("button", { name: "Workflows" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Projects" })).not.toBeInTheDocument()
  })
})
