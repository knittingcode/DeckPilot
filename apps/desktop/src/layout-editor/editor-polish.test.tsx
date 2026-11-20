// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { i18n } from "@/i18n"
import type { DeviceLayout, DeviceModel, ProfileView } from "@/lib/types"

import { deletionNotes } from "./editing"
import { LayoutEditor } from "./editor"

// Radix menus and selects rely on browser APIs that jsdom does not implement.
Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: () => Promise.resolve([]),
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve([]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: vi.fn(),
  },
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => [],
  useAppContext: () => ({
    profiles: { activeProfileId: null, profiles: [] },
    refresh: vi.fn(),
  }),
}))
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const mini: DeviceModel = {
  id: "elgato.streamdeck.mini",
  vendor: "Elgato",
  name: "Stream Deck Mini",
  usb: null,
  capabilities: {
    keys: { rows: 2, columns: 3, image: null },
    encoders: null,
    touch: null,
  },
  support: "unverified",
  verification: [],
} as unknown as DeviceModel

const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: mini.id,
  homePage: "home",
  pages: [
    {
      id: "home",
      name: "Home",
      bindings: [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "action", action: "shop" },
        },
      ],
    },
  ],
}

const view = {
  activeProjectId: null,
  profile: {
    id: "p",
    name: "Work",
    description: "",
    actions: [{ id: "shop", name: "Shop", icon: null }],
    workflows: [],
    projects: [],
    layouts: [layout],
  },
} as unknown as ProfileView

const key = (index: number) =>
  document.querySelector<HTMLButtonElement>(`[data-control="key:${index}"]`) as HTMLButtonElement

beforeEach(async () => {
  await act(async () => {
    await i18n.changeLanguage("pl")
  })
})
afterEach(async () => {
  cleanup()
  await act(async () => {
    await i18n.changeLanguage("en")
  })
})

describe("the Device Editor in Polish", { timeout: 20_000 }, () => {
  it("shows the inspector's empty state, the editing bar and the page navigation in Polish", async () => {
    const user = userEvent.setup()
    render(
      <LayoutEditor
        profileId="p"
        view={view}
        model={mini}
        saved={layout}
        onDirtyChange={vi.fn()}
        onSaved={vi.fn()}
      />,
    )
    // Nothing selected: an empty inspector, named in Polish.
    const inspector = screen.getByRole("region", { name: "Inspektor" })
    expect(inspector).toHaveTextContent("")

    const bar = screen.getByRole("toolbar", { name: "Edycja układu" })
    expect(within(bar).getByText("Wszystkie zmiany zapisane")).toBeInTheDocument()
    for (const name of ["Cofnij", "Ponów", "Przywróć", "Zapisz"]) {
      expect(within(bar).getByRole("button", { name })).toBeDisabled()
    }
    expect(screen.getByText("Strona 1 z 1")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Poprzednia strona" })).toBeDisabled()

    // An empty key: its settings in Polish, no instruction.
    await user.click(key(1))
    expect(screen.queryByText(/Przeciągnij akcję z biblioteki/)).not.toBeInTheDocument()

    // A change: unsaved, and Undo and Save are enabled.
    await user.click(key(0))
    await user.keyboard("{Delete}")
    expect(within(bar).getByText("Niezapisane")).toBeInTheDocument()
    expect(within(bar).getByRole("button", { name: "Cofnij" })).toBeEnabled()
    expect(within(bar).getByRole("button", { name: "Zapisz" })).toBeEnabled()

    // A multi-selection counts its controls with Polish plurals.
    await user.click(key(1))
    await user.keyboard("{Shift>}")
    await user.click(key(3))
    await user.keyboard("{/Shift}")
    expect(
      screen.getByRole("region", { name: "Ustawienia 3 elementów sterujących" }),
    ).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Odznacz elementy sterujące" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /Wyczyść 3 elementy sterujące/ })).toBeInTheDocument()
  })

  it("switches the editing bar's language without losing the draft", async () => {
    const user = userEvent.setup()
    render(
      <LayoutEditor
        profileId="p"
        view={view}
        model={mini}
        saved={layout}
        onDirtyChange={vi.fn()}
        onSaved={vi.fn()}
      />,
    )
    await user.click(key(0))
    await user.keyboard("{Delete}")
    expect(screen.getByText("Niezapisane")).toBeInTheDocument()
    await act(async () => {
      await i18n.changeLanguage("en")
    })
    expect(screen.getByText("Unsaved")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Undo" })).toBeEnabled()
  })
})

describe("editing bar sizing", () => {
  it("uses the toolbar height and the shared small buttons, sized by their labels in both languages", async () => {
    render(
      <LayoutEditor
        profileId="p"
        view={view}
        model={mini}
        saved={layout}
        onDirtyChange={vi.fn()}
        onSaved={vi.fn()}
      />,
    )
    for (const [language, revert, save] of [
      ["pl", "Przywróć", "Zapisz"],
      ["en", "Revert", "Save"],
    ] as const) {
      await act(async () => {
        await i18n.changeLanguage(language)
      })
      const bar = document.querySelector<HTMLElement>('[data-slot="editing-bar"]')
      if (!bar) throw new Error("no editing bar")
      expect(bar).toHaveAttribute("role", "toolbar")
      // The main toolbar's height with the border outside it: 8px around
      // the 32px buttons (measured live by scripts/layout-check.mjs).
      expect(bar).toHaveClass("h-(--toolbar-height)", "box-content")
      const buttons = within(bar).getAllByRole("button")
      for (const button of buttons) {
        // Only the shared sizes: no per-button height or width.
        expect(button.className).toMatch(/\bh-8\b|\bsize-8\b/)
        expect(button.className).not.toMatch(/\b(w|min-w|max-w)-\[?\d/)
      }
      expect(within(bar).getByRole("button", { name: revert })).toHaveClass("h-8")
      expect(within(bar).getByRole("button", { name: save })).toHaveClass("h-8")
    }
  })
})

describe("Polish plurals of the page deletion", () => {
  const deletion = (controls: number) => ({
    name: "Narzędzia",
    controls,
    openers: 0,
    inside: [],
    folder: null,
  })

  it("uses one, few and many", () => {
    expect(deletionNotes(deletion(1))).toEqual([
      "1 element sterujący na tej stronie zostanie usunięty razem z nią.",
    ])
    expect(deletionNotes(deletion(3))).toEqual([
      "3 elementy sterujące na tej stronie zostaną usunięte razem z nią.",
    ])
    expect(deletionNotes(deletion(5))).toEqual([
      "5 elementów sterujących na tej stronie zostanie usuniętych razem z nią.",
    ])
    expect(deletionNotes(deletion(22))).toEqual([
      "22 elementy sterujące na tej stronie zostaną usunięte razem z nią.",
    ])
  })
})
