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
import { appContext } from "@/test/render"

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
  return appContext({
    profiles: { profiles: [], activeProfileId: null },
    editingProfileId: null,
    modules,
    ...overrides,
  })
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
  // Tooltips and menus measure themselves; jsdom has no ResizeObserver.
  if (!("ResizeObserver" in globalThis)) {
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    )
  }
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
  it("lists the categories, marks the current one and searches", async () => {
    const value = context([projects, general])
    renderWith(value, <SettingsSidebar current="security" />)
    expect(screen.getByText("Security & Data")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Security" })).toHaveAttribute("data-active", "true")
    expect(screen.getByRole("button", { name: "Security" })).toHaveAttribute("aria-current", "page")
    // The module's own settings page.
    await userEvent.click(screen.getByRole("button", { name: "Projects" }))
    expect(value.openSettings).toHaveBeenCalledWith("module:projects")

    await userEvent.type(screen.getByLabelText("Search settings"), "keychain")
    expect(screen.getByRole("button", { name: "Security" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Appearance" })).not.toBeInTheDocument()
    expect(screen.queryByText("Hardware")).not.toBeInTheDocument()

    // One navigation: the main window's toolbar has back, forward and the
    // way out; the sidebar has none of its own.
    expect(screen.queryByRole("button", { name: "Back to DeckPilot" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Back" })).not.toBeInTheDocument()
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
  beforeEach(() => {
    localStorage.clear()
  })

  it("opens the settings in the main window with the keyboard, and leaves them for the page before", () => {
    const { result } = renderHook(() => useNavigation(() => true))
    // The device editor first.
    expect(result.current.location).toEqual({ mode: "workspace", page: "designer" })
    act(() => {
      result.current.navigate("workflows")
    })
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: ",", ctrlKey: true }))
    })
    // The settings are a location of the main window, at the default page.
    expect(result.current.location).toEqual({ mode: "settings", page: "profiles" })
    // The workspace page they return to (and that stays mounted under them).
    expect(result.current.workspace).toBe("workflows")
    // No back and forward: neither shortcuts nor mouse buttons move.
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", altKey: true }))
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "[", metaKey: true }))
      window.dispatchEvent(new MouseEvent("mouseup", { button: 3 }))
    })
    expect(result.current.location).toEqual({ mode: "settings", page: "profiles" })
    expect(result.current).not.toHaveProperty("back")
    act(() => {
      result.current.closeSettings()
    })
    expect(result.current.location).toEqual({ mode: "workspace", page: "workflows" })
  })

  it("restores the location of the last session", () => {
    const first = renderHook(() => useNavigation(() => true))
    act(() => {
      first.result.current.navigate("actions")
    })
    first.unmount()
    const second = renderHook(() => useNavigation(() => true))
    expect(second.result.current.location).toEqual({ mode: "workspace", page: "actions" })
  })

  it("opens settings pages, and pages that moved there, in the main window", () => {
    const { result } = renderHook(() => useNavigation(() => true))
    act(() => {
      result.current.navigate("dashboard")
    })
    const references: [string, string][] = [
      ["models", "devices"],
      ["devices", "devices"],
      ["profiles", "profiles"],
      ["settings:security", "security"],
    ]
    for (const [reference, page] of references) {
      act(() => {
        result.current.navigate(reference)
      })
      expect(result.current.location).toEqual({ mode: "settings", page })
    }
    act(() => {
      result.current.go({ mode: "settings", page: "updates" })
    })
    expect(result.current.location).toEqual({ mode: "settings", page: "updates" })
    expect(result.current.workspace).toBe("dashboard")
  })

  it("returns to the settings page shown last, also in a later session", () => {
    const first = renderHook(() => useNavigation(() => true))
    act(() => {
      first.result.current.openSettings("security")
    })
    act(() => {
      first.result.current.navigate("designer")
    })
    act(() => {
      first.result.current.openSettings()
    })
    expect(first.result.current.location).toEqual({ mode: "settings", page: "security" })
    first.unmount()
    // A new history (e.g. it was cleared) still opens where the user was.
    localStorage.removeItem("deckpilot.navigation")
    const second = renderHook(() => useNavigation(() => true))
    expect(second.result.current.location).toEqual({ mode: "workspace", page: "designer" })
    act(() => {
      second.result.current.openSettings()
    })
    expect(second.result.current.location).toEqual({ mode: "settings", page: "security" })
    // An explicit page wins.
    act(() => {
      second.result.current.openSettings("devices")
    })
    expect(second.result.current.location).toEqual({ mode: "settings", page: "devices" })
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
    expect(result.current.location).toEqual({ mode: "workspace", page: "designer" })
  })
})
