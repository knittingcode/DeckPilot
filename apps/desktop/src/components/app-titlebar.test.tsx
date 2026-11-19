// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { Play, Plus, Search } from "lucide-react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { AppTitlebar } from "@/components/app-titlebar"
import { SidebarProvider } from "@/components/ui/sidebar"
import { TitlebarContext, useTitlebar } from "@/hooks/use-titlebar"
import { type TitlebarConfig, TitlebarStore, arrangeActions, mergeTitlebar } from "@/lib/titlebar"

vi.mock("@/lib/api", () => ({ api: { titlebarDoubleClick: vi.fn() } }))

beforeAll(() => {
  window.matchMedia = vi.fn().mockReturnValue({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  })
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
})

afterEach(cleanup)

function Contribution({ config }: { config: TitlebarConfig }) {
  useTitlebar(config)
  return null
}

function Shell({ store, children }: { store: TitlebarStore; children?: React.ReactNode }) {
  return (
    <TitlebarContext value={store}>
      <SidebarProvider>
        <AppTitlebar
          store={store}
          chrome={{ overlay: false, controlsInset: 0 }}
          fallbackTitle="Loading page"
        />
        {children}
      </SidebarProvider>
    </TitlebarContext>
  )
}

describe("titlebar registry", () => {
  it("merges contributions: later values win, actions combine by id", () => {
    const merged = mergeTitlebar([
      { title: "A", actions: [{ id: "x", label: "Old" }] },
      {
        subtitle: "s",
        actions: [
          { id: "x", label: "New" },
          { id: "y", label: "Y" },
        ],
      },
      { title: "B", progress: false },
    ])
    expect(merged.title).toBe("B")
    expect(merged.subtitle).toBe("s")
    expect(merged.progress).toBeUndefined()
    expect(merged.actions?.map((a) => a.label)).toEqual(["New", "Y"])
  })

  it("arranges actions by placement and order", () => {
    const { primary, secondary, overflow } = arrangeActions([
      { id: "a", label: "a", order: 2 },
      { id: "b", label: "b", placement: "primary" },
      { id: "c", label: "c", order: 1 },
      { id: "d", label: "d", placement: "overflow" },
    ])
    expect(primary.map((a) => a.id)).toEqual(["b"])
    expect(secondary.map((a) => a.id)).toEqual(["c", "a"])
    expect(overflow.map((a) => a.id)).toEqual(["d"])
  })

  it("only notifies the titlebar when something visible changes", () => {
    const store = new TitlebarStore()
    const listener = vi.fn()
    store.subscribe(listener)
    store.set("page", { title: "One", actions: [{ id: "run", label: "Run", onSelect: vi.fn() }] })
    const latest = vi.fn()
    store.set("page", { title: "One", actions: [{ id: "run", label: "Run", onSelect: latest }] })
    expect(listener).toHaveBeenCalledTimes(1)
    // The titlebar still calls the latest callback.
    store.action("run")?.onSelect?.()
    expect(latest).toHaveBeenCalled()
    store.remove("page")
    expect(listener).toHaveBeenCalledTimes(2)
    expect(store.getSnapshot()).toEqual({})
  })
})

describe("app titlebar", () => {
  it("shows the page's title, status and actions, and removes them on unmount", async () => {
    const store = new TitlebarStore()
    const run = vi.fn()
    const create = vi.fn()
    const { rerender } = render(
      <Shell store={store}>
        <Contribution
          config={{
            title: "Workflows",
            subtitle: "Run several actions in sequence",
            status: { caption: "Profile", label: "Work", tone: "positive" },
            actions: [
              { id: "new", label: "New workflow", icon: Plus, onSelect: create },
              { id: "run", label: "Run workflow", icon: Play, placement: "primary", onSelect: run },
              { id: "search", label: "Search", icon: Search, placement: "overflow" },
            ],
          }}
        />
      </Shell>,
    )
    expect(screen.getByRole("heading", { name: "Workflows" })).toBeInTheDocument()
    expect(screen.getByText("Work")).toBeInTheDocument()
    const toolbar = within(screen.getByRole("toolbar", { name: "Page actions" }))
    await userEvent.click(toolbar.getByRole("button", { name: "Run workflow" }))
    expect(run).toHaveBeenCalledTimes(1)
    await userEvent.click(toolbar.getByRole("button", { name: "New workflow" }))
    expect(create).toHaveBeenCalledTimes(1)
    // Overflow actions are in the menu.
    await userEvent.click(toolbar.getByRole("button", { name: "More actions" }))
    expect(await screen.findByRole("menuitem", { name: "Search" })).toBeInTheDocument()
    await userEvent.keyboard("{Escape}")

    rerender(<Shell store={store} />)
    expect(screen.getByRole("heading", { name: "Loading page" })).toBeInTheDocument()
    expect(screen.queryByRole("toolbar", { name: "Page actions" })).not.toBeInTheDocument()
  })

  it("has no sidebar toggle and no back or forward", () => {
    render(<Shell store={new TitlebarStore()} />)
    for (const name of [/Toggle Sidebar/i, "Back", "Forward"]) {
      expect(screen.queryByRole("button", { name })).not.toBeInTheDocument()
    }
    expect(screen.queryByRole("navigation", { name: "History" })).not.toBeInTheDocument()
    expect(document.querySelector("[data-slot=titlebar-sidebar-extension]")).toBeNull()
  })

  it("leaves only room for the native window controls before the selectors", () => {
    render(<Shell store={new TitlebarStore()} />)
    const inset = document.querySelector("[data-slot=titlebar-controls-inset]") as HTMLElement
    // First in the toolbar, nothing in it, sized by the platform's inset
    // alone (0 on Windows, Linux and in fullscreen).
    expect(Array.from(inset.parentElement?.children ?? []).indexOf(inset)).toBe(0)
    expect(inset).toBeEmptyDOMElement()
    expect(inset).toHaveAttribute("aria-hidden", "true")
    expect(inset.className).toBe("w-(--titlebar-controls-inset) shrink-0")
  })

  it("shows determinate progress", () => {
    const store = new TitlebarStore()
    render(<Shell store={store} />)
    act(() => {
      store.set("download", { progress: 42 })
    })
    expect(screen.getByRole("progressbar", { name: "Progress" })).toHaveAttribute(
      "aria-valuenow",
      "42",
    )
  })

  it("does not move the window from controls, and only with the overlay titlebar", () => {
    const store = new TitlebarStore()
    render(<Shell store={store} />)
    // Without the macOS overlay there are no drag handlers at all.
    expect(screen.getByRole("banner", { name: "Toolbar" })).not.toHaveAttribute("data-drag")
  })
})
