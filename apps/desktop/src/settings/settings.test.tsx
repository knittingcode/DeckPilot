// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, renderHook, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { SettingsSidebar } from "@/components/settings-sidebar"
import { SidebarProvider } from "@/components/ui/sidebar"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useNavigation } from "@/hooks/use-navigation"
import { AppContext, type AppContextValue } from "@/lib/app-context"
import { moduleInfo } from "@/lib/test-fixtures"
import type { ModuleInfo } from "@/lib/types"
import { ModulesSettings } from "@/settings/pages/modules"
import { groupedSettings, settingsPages } from "@/settings/registry"

const api = { setModuleEnabled: vi.fn() }

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, name: string) => api[name as keyof typeof api] }),
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))

const projects = moduleInfo({
  id: "projects",
  name: "Projects",
  config: [
    {
      key: "root",
      label: "Projects folder",
      kind: { type: "text", multiline: false },
      required: false,
    },
  ],
  pages: [{ id: "projects", title: "Projects", icon: "folder-kanban", order: 10 }],
})
const general = moduleInfo({ id: "general", name: "General", capabilities: ["profile-read"] })
const timer = moduleInfo({ id: "timer", name: "Timer", dependencies: ["general"] })

function context(modules: ModuleInfo[], overrides: Partial<AppContextValue> = {}): AppContextValue {
  return {
    profiles: { profiles: [], activeProfileId: null },
    editingProfileId: null,
    setEditingProfileId: vi.fn(),
    refresh: vi.fn(),
    revision: 0,
    agent: null,
    setAgent: vi.fn(),
    updates: null,
    setUpdates: vi.fn(),
    navigate: vi.fn(),
    openSettings: vi.fn(),
    closeSettings: vi.fn(),
    history: { canGoBack: false, canGoForward: false, back: vi.fn(), forward: vi.fn() },
    modules,
    setModules: vi.fn(),
    ...overrides,
  }
}

function renderWith(value: AppContextValue, ui: React.ReactNode) {
  return render(
    <AppContext value={value}>
      <TooltipProvider>
        <SidebarProvider>{ui}</SidebarProvider>
      </TooltipProvider>
    </AppContext>,
  )
}

beforeAll(() => {
  window.matchMedia = vi.fn().mockReturnValue({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  })
})

beforeEach(() => {
  api.setModuleEnabled.mockReset()
  localStorage.clear()
})

afterEach(cleanup)

describe("settings registry", () => {
  it("groups pages into categories and leaves empty categories out", () => {
    const groups = groupedSettings(settingsPages([]))
    expect(groups.map((g) => g.label)).toEqual([
      "General",
      "Hardware",
      "Security & Data",
      "Extensions",
      "Advanced",
    ])
    expect(groups.find((g) => g.id === "hardware")?.pages.map((p) => p.title)).toEqual([
      "Devices",
      "Background Agent",
    ])
    const onlyGeneral = groupedSettings(settingsPages([]).filter((p) => p.category === "general"))
    expect(onlyGeneral.map((g) => g.id)).toEqual(["general"])
  })

  it("adds a settings page for enabled modules that declare settings", () => {
    const ids = (modules: ModuleInfo[]) =>
      settingsPages(modules)
        .filter((p) => p.module)
        .map((p) => p.id)
    expect(ids([projects, general])).toEqual(["module:projects"])
    expect(ids([{ ...projects, enabled: false }, general])).toEqual([])
  })
})

describe("settings sidebar", () => {
  it("lists the categories, searches and returns to the workspace", async () => {
    const value = context([projects, general])
    renderWith(value, <SettingsSidebar current="security" />)
    expect(screen.getByText("Security & Data")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Security" })).toHaveAttribute("data-active", "true")
    // The module's own settings page.
    await userEvent.click(screen.getByRole("button", { name: "Projects" }))
    expect(value.openSettings).toHaveBeenCalledWith("module:projects")

    await userEvent.type(screen.getByLabelText("Search settings"), "keychain")
    expect(screen.getByRole("button", { name: "Security" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Appearance" })).not.toBeInTheDocument()
    expect(screen.queryByText("Hardware")).not.toBeInTheDocument()

    await userEvent.click(screen.getByRole("button", { name: "Back to DeckPilot" }))
    expect(value.closeSettings).toHaveBeenCalled()
  })
})

describe("modules page", () => {
  it("explains why a module cannot be switched and enables others", async () => {
    const value = context([general, timer, { ...projects, enabled: false }])
    api.setModuleEnabled.mockResolvedValue([general, timer, projects])
    renderWith(value, <ModulesSettings />)

    const generalSwitch = screen.getByRole("switch", { name: "Disable General" })
    expect(generalSwitch).toBeDisabled()
    expect(generalSwitch).toHaveAccessibleDescription("Required by Timer. Disable it first.")

    await userEvent.click(screen.getByRole("switch", { name: "Enable Projects" }))
    expect(api.setModuleEnabled).toHaveBeenCalledWith("projects", true)
    await waitFor(() => {
      expect(value.setModules).toHaveBeenCalledWith([general, timer, projects])
    })
    expect(value.refresh).toHaveBeenCalled()
  })

  it("searches and filters modules", async () => {
    renderWith(context([general, timer, { ...projects, enabled: false }]), <ModulesSettings />)
    const list = () => within(screen.getByRole("list", { name: "Modules" }))

    await userEvent.click(screen.getByRole("radio", { name: /Disabled/ }))
    expect(list().getAllByRole("listitem")).toHaveLength(1)
    expect(list().getByText("Projects")).toBeInTheDocument()

    await userEvent.click(screen.getByRole("radio", { name: /All/ }))
    await userEvent.type(screen.getByLabelText("Search modules"), "tim")
    expect(list().getAllByRole("listitem")).toHaveLength(1)
    expect(list().getByText("Timer")).toBeInTheDocument()

    await userEvent.clear(screen.getByLabelText("Search modules"))
    await userEvent.type(screen.getByLabelText("Search modules"), "nothing like this")
    expect(screen.getByText("No modules found")).toBeInTheDocument()
  })

  it("opens a module's settings and shows its details", async () => {
    const value = context([projects, general])
    renderWith(value, <ModulesSettings />)
    const item = within(screen.getByText("Projects").closest("li") as HTMLElement)
    await userEvent.click(item.getByRole("button", { name: "Settings" }))
    expect(value.openSettings).toHaveBeenCalledWith("module:projects")
    // General declares no settings: no settings button.
    const other = within(screen.getByText("General").closest("li") as HTMLElement)
    expect(other.queryByRole("button", { name: "Settings" })).not.toBeInTheDocument()
    await userEvent.click(other.getByRole("button", { name: "Details" }))
    expect(other.getByText("Read the active profile")).toBeInTheDocument()
  })
})

describe("navigation hook", () => {
  it("opens settings and goes back with the keyboard and mouse buttons", () => {
    const { result } = renderHook(() => useNavigation(() => true))
    act(() => {
      result.current.navigate("workflows")
    })
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: ",", ctrlKey: true }))
    })
    expect(result.current.location).toEqual({ mode: "settings", page: "appearance" })
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", altKey: true }))
    })
    expect(result.current.location).toEqual({ mode: "workspace", page: "workflows" })
    act(() => {
      window.dispatchEvent(new MouseEvent("mouseup", { button: 4 }))
    })
    expect(result.current.location).toEqual({ mode: "settings", page: "appearance" })
    act(() => {
      result.current.closeSettings()
    })
    expect(result.current.location).toEqual({ mode: "workspace", page: "workflows" })
  })

  it("restores the location of the last session", () => {
    const first = renderHook(() => useNavigation(() => true))
    act(() => {
      first.result.current.openSettings("storage")
    })
    first.unmount()
    const second = renderHook(() => useNavigation(() => true))
    expect(second.result.current.location).toEqual({ mode: "settings", page: "storage" })
    expect(second.result.current.canGoBack).toBe(true)
  })

  it("leaves pages that disappear, e.g. of a disabled module", () => {
    let exists: (location: { page: string }) => boolean = () => true
    const { result, rerender } = renderHook(({ check }) => useNavigation(check), {
      initialProps: { check: exists },
    })
    act(() => {
      result.current.navigate("projects/projects")
    })
    exists = (location) => location.page !== "projects/projects"
    rerender({ check: exists })
    expect(result.current.location).toEqual({ mode: "workspace", page: "dashboard" })
    expect(result.current.canGoBack).toBe(false)
  })
})
