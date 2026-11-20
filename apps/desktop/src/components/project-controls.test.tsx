// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// Project controls come with the Projects plugin (or its legacy module
// before the migration); without it what is stored stays, read-only.

import { cleanup, screen } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { ParameterField } from "@/components/parameter-field"
import { pluginModules } from "@/lib/plugin-actions"
import { moduleInfo } from "@/lib/test-fixtures"
import type { ParameterSpec } from "@/lib/types"
import { installDomStubs } from "@/test/dom"
import { project } from "@/test/fixtures"
import { projectsPlugin } from "@/test/first-party"
import { renderWithApp } from "@/test/render"
import { ProjectScopeSetting } from "@/workflow-builder/project-scope"

beforeAll(installDomStubs)
afterEach(() => {
  cleanup()
  vi.unstubAllEnvs()
})

const spec: ParameterSpec = {
  key: "project",
  label: "Project",
  kind: { type: "project" },
  required: true,
}
const shop = project("SHOP", { name: "Online Shop" })

function field() {
  return (
    <ParameterField
      spec={spec}
      value={{ type: "project", value: shop.id }}
      projects={[shop]}
      onChange={vi.fn()}
    />
  )
}

function scope() {
  return <ProjectScopeSetting scope="application" selectsProjects onChange={vi.fn()} />
}

describe("project controls", () => {
  it("are read-only, with the reason, while no plugin provides projects", () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    renderWithApp(
      <>
        {field()}
        {scope()}
      </>,
      { modules: [], pluginModules: [] },
    )
    expect(screen.getByText("SHOP · Online Shop")).toBeInTheDocument()
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument()
    expect(screen.getByRole("note")).toHaveTextContent("The Projects plugin is off")
    // The workflow's project permission is not offered (and is kept as stored).
    expect(screen.queryByRole("switch")).not.toBeInTheDocument()
  })

  it("are editable while the Projects plugin is enabled, and live when it is disabled", () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const { rerender, context } = renderWithApp(
      <>
        {field()}
        {scope()}
      </>,
      { modules: [], pluginModules: pluginModules([projectsPlugin()]) },
    )
    expect(screen.getByRole("combobox")).toBeInTheDocument()
    expect(screen.getByRole("switch")).toBeChecked()
    rerender(
      <>
        {field()}
        {scope()}
      </>,
      { ...context, pluginModules: pluginModules([projectsPlugin("disabled")]) },
    )
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument()
    expect(screen.queryByRole("switch")).not.toBeInTheDocument()
  })

  it("follow the legacy Projects module in builds without plugins", () => {
    renderWithApp(field(), { modules: [moduleInfo({ id: "projects" })] })
    expect(screen.getByRole("combobox")).toBeInTheDocument()
    cleanup()
    renderWithApp(field(), { modules: [moduleInfo({ id: "projects", enabled: false })] })
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument()
  })

  it("explains in Polish", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const { i18n } = await import("@/i18n")
    await i18n.changeLanguage("pl")
    try {
      renderWithApp(field(), { modules: [], pluginModules: [] })
      expect(screen.getByRole("note")).toHaveTextContent("Wtyczka Projekty jest wyłączona")
    } finally {
      await i18n.changeLanguage("en")
    }
  })
})
