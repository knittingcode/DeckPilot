// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// The device editor without optional modules: no project selector of its
// own, previews drawn for the display, the library always beside it, and
// the editor's place kept by the shell.

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { displayPixels } from "@/hooks/use-device-pixel-ratio"
import type { ControlFace, DeviceLayout, DeviceModel, ProfileView } from "@/lib/types"
import { EditorSelectionContext, type EditorSelection } from "@/shell/editor-selection-context"
import { currentPage, goToPage } from "@/test/pages"

import { LayoutEditor } from "./editor"
import type { EditorPlace } from "./use-layout-editing"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const previewLayout = vi.fn<(...args: unknown[]) => Promise<ControlFace[]>>()

vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: (...args: unknown[]) => previewLayout(...args),
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve([]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: vi.fn(),
  },
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("@/lib/app-context", () => ({
  // No modules at all: Projects disabled or not installed.
  useActionModules: () => [],
  useAppContext: () => ({
    profiles: { activeProfileId: null, profiles: [] },
    refresh: vi.fn(),
  }),
}))
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: {
    keys: { rows: 3, columns: 5, image: { width: 72, height: 72 } as never },
    encoders: null,
    touch: null,
  },
  support: "verified",
  verification: [],
}

const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: mk2.id,
  homePage: "home",
  pages: [
    {
      id: "home",
      name: "Home",
      bindings: [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "action", action: "switch" },
          label: "Work on {project.key}",
        },
      ],
    },
    { id: "second", name: "Second", bindings: [] },
  ],
}

/** A profile with a project-aware action; `active`: its active project. */
function profileView(active: string | null, projects = true): ProfileView {
  return {
    activeProjectId: active,
    profile: {
      id: "p",
      name: "Work",
      description: "",
      actions: [
        {
          id: "switch",
          name: "Switch project",
          icon: "folder-kanban",
          actionType: "projects.switch",
          parameters: { project: "shop" },
        },
      ],
      workflows: [],
      projects: projects
        ? [
            { id: "shop", key: "SHOP", name: "Shop" },
            { id: "blog", key: "BLOG", name: "Blog" },
          ]
        : [],
      layouts: [layout],
    },
  } as unknown as ProfileView
}

function renderEditor(view: ProfileView, shell?: EditorSelection) {
  const user = userEvent.setup()
  const editor = (
    <LayoutEditor
      profileId="p"
      view={view}
      model={mk2}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />
  )
  const result = render(
    shell ? <EditorSelectionContext value={shell}>{editor}</EditorSelectionContext> : editor,
  )
  return { user, ...result }
}

/** A shell that remembers places as the real one does. */
function shellSelection(places = new Map<string, EditorPlace>()): EditorSelection {
  return {
    models: { status: "ready", data: [mk2] },
    modelId: mk2.id,
    device: null,
    layoutId: null,
    selectDevice: vi.fn(() => true),
    selectProfile: vi.fn(() => true),
    selectLayout: vi.fn(() => true),
    showLayout: vi.fn(),
    adoptModel: vi.fn(),
    setDirty: vi.fn(),
    guard: vi.fn(() => true),
    isDirty: vi.fn(() => false),
    place: (key) => places.get(key),
    rememberPlace: (key, place) => {
      places.set(key, place)
    },
  }
}

const key = (name: string) => screen.getByRole("button", { name })

beforeEach(() => {
  previewLayout.mockReset()
  previewLayout.mockResolvedValue([])
})
afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe("device editor without a project selector", { timeout: 20_000 }, () => {
  it("has no project selector or explanation, with Projects disabled or uninstalled", async () => {
    renderEditor(profileView(null, false))
    expect(screen.queryByText("Preview with project")).not.toBeInTheDocument()
    expect(screen.queryByText(/follow the selected project/)).not.toBeInTheDocument()
    expect(screen.queryByRole("combobox", { name: /project/i })).not.toBeInTheDocument()
    expect(document.getElementById("preview-project")).toBeNull()
    // The editor works: keys, library and page navigation.
    expect(screen.getAllByRole("button", { name: /^Key \d+$/ })).toHaveLength(15)
    expect(screen.getByRole("complementary", { name: "Action Library" })).toBeInTheDocument()
    expect(currentPage()).toBe("Home")
    // Without projects, project labels show a dash, as on the device.
    expect(key("Key 1")).toHaveAttribute("title", "Key 1: Press: Work on -")
    // The preview asks for the profile's own project context (none here).
    await waitFor(() => {
      expect(previewLayout).toHaveBeenCalled()
    })
    expect(previewLayout.mock.calls[0]?.[3]).toBeNull()
  })

  it("shows labels with the profile's active project, as the device does", async () => {
    renderEditor(profileView("blog"))
    expect(screen.queryByText("Preview with project")).not.toBeInTheDocument()
    expect(key("Key 1")).toHaveAttribute("title", "Key 1: Press: Work on BLOG")
    // The backend resolves the profile's active project itself (null: no
    // choice of the editor's), so profiles keep their own projects.
    await waitFor(() => {
      expect(previewLayout).toHaveBeenCalled()
    })
    expect(previewLayout.mock.calls.every((call) => call[3] === null)).toBe(true)
  })

  it("keeps the project action's configuration in the inspector", async () => {
    const { user } = renderEditor(profileView("shop"))
    await user.click(key("Key 1"))
    // The action and its project stay as configured; nothing in the editor
    // changed them.
    expect(screen.getAllByText("Switch project").length).toBeGreaterThan(0)
    expect(
      previewLayout.mock.calls.every((call) => JSON.stringify(call[1]) === JSON.stringify(layout)),
    ).toBe(true)
  })
})

describe("previews drawn for the display", { timeout: 20_000 }, () => {
  it("asks the renderer for the size a key is shown at, times the pixel ratio", async () => {
    vi.stubGlobal("devicePixelRatio", 2)
    renderEditor(profileView(null))
    // jsdom measures nothing: the fallback key size, 72 CSS px, less the
    // key's border on both sides (4px each at that size), on a 2x display.
    await waitFor(() => {
      expect(previewLayout).toHaveBeenCalled()
    })
    expect(previewLayout.mock.calls[0]?.slice(4)).toEqual([true, null, 128])
  })

  it("asks for the CSS size on a 1x display, and never for a fraction", () => {
    expect(displayPixels(70, 1)).toBe(70)
    expect(displayPixels(70, 2)).toBe(140)
    expect(displayPixels(102, 1.5)).toBe(153)
    expect(displayPixels(70, 1.25)).toBe(88)
    expect(displayPixels(70, Number.NaN)).toBe(70)
    expect(displayPixels(0, 2)).toBe(0)
  })

  it("shows rendered images edge to edge, empty keys in the app's own tone", async () => {
    previewLayout.mockResolvedValue([
      {
        control: { kind: "key", index: 0 },
        face: { style: "action", title: "Switch project" },
        image: "data:image/png;base64,sharp",
      },
      {
        control: { kind: "key", index: 1 },
        face: { style: "empty", title: "" },
        image: "data:image/png;base64,black",
      },
    ] as ControlFace[])
    renderEditor(profileView(null))
    await waitFor(() => {
      expect(key("Key 1").querySelector("[data-slot=key-image]")).toHaveAttribute(
        "src",
        "data:image/png;base64,sharp",
      )
    })
    const image = key("Key 1").querySelector("img") as HTMLImageElement
    // Neither pixelated nor filtered: the image is already the right size.
    expect(image.className).not.toMatch(/pixelated|blur|contrast|sharpen/)
    expect(image.style.imageRendering).toBe("")
    expect(key("Key 2").querySelector("img")).toBeNull()
    expect(key("Key 2").querySelector("[data-face=empty]")).toBeInTheDocument()
  })
})

describe("dark appearance", { timeout: 20_000 }, () => {
  it("draws keys and the library from tokens, the controls without an enclosure", async () => {
    previewLayout.mockResolvedValue([
      {
        control: { kind: "key", index: 0 },
        face: { style: "action", title: "Switch project" },
        image: "data:image/png;base64,key",
      },
    ] as ControlFace[])
    document.documentElement.classList.add("dark")
    renderEditor(profileView(null))
    await waitFor(() => {
      expect(key("Key 1").querySelector("img")).not.toBeNull()
    })
    // The key image is what the device shows, unchanged by the editor.
    expect(key("Key 1").querySelector("img")).toHaveAttribute("src", "data:image/png;base64,key")
    // Tokens from index.css; no fixed colours.
    expect(key("Key 2").className).toContain("bg-device-key")
    expect(key("Key 2").className).not.toMatch(/bg-(black|white|neutral|zinc|gray)/)
    const library = screen.getByRole("complementary", { name: "Action Library" })
    expect(library.className).toContain("bg-sidebar")
    // The controls sit directly on the workspace: no imitation of the
    // device's body (background, border, rounded enclosure).
    const controls = screen.getByRole("group", { name: "Stream Deck MK.2 controls" })
    expect(controls.className).not.toMatch(/\b(bg-|border|rounded|shadow)/)
    document.documentElement.classList.remove("dark")
  })
})

describe("the library and the editor's place", { timeout: 20_000 }, () => {
  it("keeps the library, at its fixed width, while pages and controls change", async () => {
    const { user } = renderEditor(profileView(null))
    const library = screen.getByRole("complementary", { name: "Action Library" })
    const before = library.className
    await user.click(key("Key 1"))
    await goToPage(user, "Second")
    await user.click(key("Key 5"))
    const after = screen.getByRole("complementary", { name: "Action Library" })
    expect(after).toBe(library)
    expect(after.className).toBe(before)
    expect(after.style.width).toBe("")
    expect(within(after).queryByRole("button", { name: /collapse|show the library/i })).toBeNull()
    expect(screen.queryByRole("separator", { name: /resize/i })).toBeNull()
  })

  it("comes back on the page and with the keys it showed", async () => {
    const places = new Map<string, EditorPlace>()
    const first = renderEditor(profileView(null), shellSelection(places))
    await goToPage(first.user, "Second")
    await first.user.click(key("Key 3"))
    expect(key("Key 3")).toHaveAttribute("aria-pressed", "true")
    // The editor leaves the screen (another view) and comes back.
    act(() => {
      first.unmount()
    })
    renderEditor(profileView(null), shellSelection(places))
    expect(currentPage()).toBe("Second")
    expect(key("Key 3")).toHaveAttribute("aria-pressed", "true")
    expect(key("Key 1")).toHaveAttribute("aria-pressed", "false")
  })

  it("opens the home page when the remembered page is gone", () => {
    const places = new Map<string, EditorPlace>([
      [
        "p:layout",
        {
          pageId: "deleted",
          trail: [],
          selection: { controls: [{ kind: "key", index: 2 }], anchor: null, focus: null },
        },
      ],
    ])
    renderEditor(profileView(null), shellSelection(places))
    expect(currentPage()).toBe("Home")
    expect(key("Key 3")).toHaveAttribute("aria-pressed", "false")
  })
})
