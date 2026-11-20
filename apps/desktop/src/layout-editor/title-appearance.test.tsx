// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { i18n } from "@/i18n"
import { installDomStubs } from "@/test/dom"
import type {
  ActionType,
  ControlBinding,
  ControlFace,
  DeviceLayout,
  DeviceModel,
  KeyAppearance,
  ProfileView,
  TextLayoutReport,
} from "@/lib/types"

import { LayoutEditor } from "./editor"

const api = {
  previewLayout: vi.fn(),
  previewMarquee: vi.fn(),
  saveLayout: vi.fn(),
  listIcons: vi.fn(),
  importIcon: vi.fn(),
  deleteIcon: vi.fn(),
  pageTree: vi.fn(),
  listActionTypes: vi.fn(),
  getGestureDefaults: vi.fn(),
  titleFonts: vi.fn(),
}

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, name: string) => api[name as keyof typeof api] }),
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => [],
  useAppContext: () => ({
    profiles: { activeProfileId: null, profiles: [] },
    refresh: vi.fn(),
  }),
}))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }))

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: {
    keys: { rows: 3, columns: 5, image: { width: 72, height: 72, format: "jpeg" } },
    encoders: null,
    touch: null,
  },
  support: "unverified",
  verification: [],
  defaultAppearance: {
    keyBackground: "transparent",
    textColor: "#ffffff",
    displayBackground: "#000000",
  },
}

const press = (index: number, action: string, appearance?: KeyAppearance, label?: string) =>
  ({
    control: { kind: "key", index },
    trigger: { type: "press" },
    target: { type: "action", action },
    label: label ?? null,
    ...(appearance ? { appearance } : {}),
  }) as ControlBinding

/** A plugin's action type that offers only the title's position and colour. */
const cpuType = {
  id: "acme.system.cpu",
  name: "CPU",
  description: "",
  category: "System",
  parameters: [],
  requiresProject: false,
  title: { default: "CPU", dynamic: true, position: "top", supports: ["position", "color"] },
} as unknown as ActionType

function makeLayout(): DeviceLayout {
  return {
    id: "layout",
    name: "Main",
    model: mk2.id,
    homePage: "home",
    explicitTree: true,
    pages: [
      {
        id: "home",
        name: "Home",
        bindings: [
          press(0, "deploy", undefined, "Deploy"),
          press(1, "mail", { textColor: "#ff0000", bold: true, titleAlign: "top" }),
          {
            control: { kind: "key", index: 2 },
            trigger: { type: "press" },
            target: { type: "openPage", page: "tools" },
            label: null,
          },
          press(3, "cpu"),
        ],
      },
      { id: "tools", name: "Tools", parent: "home", bindings: [], back: { position: "reserved" } },
    ],
  }
}

const view = {
  activeProjectId: null,
  profile: {
    id: "p",
    name: "Work",
    description: "",
    actions: [
      ...["deploy", "mail"].map((id) => ({ id, name: id, icon: null, actionType: "test.run" })),
      { id: "cpu", name: "CPU", icon: null, actionType: cpuType.id },
    ],
    workflows: [],
    projects: [],
    layouts: [makeLayout()],
  },
} as unknown as ProfileView

/** Reports by key index, as the renderer would make them. */
let reports: Record<number, TextLayoutReport> = {}

/** A fake renderer: each image names its key and its look. */
function rendered(_: string, draft: DeviceLayout, pageId: string) {
  const page = draft.pages.find((p) => p.id === pageId)
  return Promise.resolve(
    (page?.bindings ?? []).map((b): ControlFace => ({
      control: b.control,
      face: { style: "action", title: b.label ?? "Default title", symbol: "zap" },
      image: `data:image/png;base64,key${String(b.control.index)}-${JSON.stringify(b.appearance ?? {})}`,
      ...(reports[b.control.index] ? { text: reports[b.control.index] } : {}),
    })),
  )
}

const report = (patch: Partial<TextLayoutReport>): TextLayoutReport => ({
  mode: "wrap",
  fits: true,
  truncated: false,
  fontSize: 11.2,
  lines: 1,
  needsScroll: false,
  ...patch,
})

let reducedMotion = false

beforeAll(() => {
  installDomStubs()
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: query.includes("reduced-motion") ? reducedMotion : false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }))
})
beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset()
  reports = {}
  reducedMotion = false
  api.previewLayout.mockImplementation(rendered)
  api.previewMarquee.mockResolvedValue(null)
  api.pageTree.mockResolvedValue({ pages: [], warnings: [] })
  api.listIcons.mockResolvedValue([])
  api.saveLayout.mockImplementation((_, saved) =>
    Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
  )
  api.listActionTypes.mockResolvedValue([cpuType])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 300, longPressMs: 700 })
  api.titleFonts.mockResolvedValue([{ id: "inter", name: "Inter" }])
})
afterEach(cleanup)

const onDirtyChange = vi.fn()

function renderEditor() {
  const user = userEvent.setup({ applyAccept: false })
  onDirtyChange.mockReset()
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={mk2}
      saved={makeLayout()}
      onDirtyChange={onDirtyChange}
      onSaved={vi.fn()}
    />,
  )
  return user
}

const control = (name: string) => screen.getByRole("button", { name })
const popover = () => within(screen.getByRole("dialog", { name: "Title appearance" }))

async function open(user: ReturnType<typeof userEvent.setup>, key: string) {
  // Once the renderer has drawn the key (its face says what it shows).
  await waitFor(() => {
    expect(control(key).querySelector("img")).not.toBeNull()
  })
  await user.click(control(key))
  await user.click(screen.getByRole("button", { name: "Title appearance" }))
  return popover()
}

async function saved(user: ReturnType<typeof userEvent.setup>, index: number) {
  await user.keyboard("{Escape}")
  await user.click(screen.getByRole("button", { name: /Save/ }))
  const [, layout] = api.saveLayout.mock.calls.at(-1) as [string, DeviceLayout]
  return layout.pages[0]?.bindings.find((b) => b.control.index === index)
}

describe("the title appearance popover", { timeout: 20_000 }, () => {
  it("opens from the T button without changing anything, in a portal above the Inspector", async () => {
    const user = renderEditor()
    const rows = await open(user, "Key 2")
    // Rendered outside the Inspector (a portal), so its curtain and size are untouched.
    const dialog = screen.getByRole("dialog", { name: "Title appearance" })
    expect(dialog.closest("[data-slot=inspector]")).toBeNull()
    expect(dialog).toHaveAttribute("data-slot", "title-popover")
    // Row 1: position and reset; then the font; size, style, colour;
    // layout. "Show title" is the key header's, not the popover's.
    expect(rows.queryByRole("checkbox", { name: "Show title" })).toBeNull()
    expect(rows.queryByRole("switch", { name: "Show title" })).toBeNull()
    expect(screen.getByRole("switch", { name: "Show title" })).toBeChecked()
    expect(rows.getByRole("radiogroup", { name: "Title position" })).toBeInTheDocument()
    expect(rows.getByRole("button", { name: "Reset title appearance" })).toBeEnabled()
    expect(rows.getByRole("combobox", { name: "Font" })).toHaveTextContent("Inter")
    expect(rows.getByRole("spinbutton", { name: "Size (px)" })).toHaveAttribute("placeholder", "11")
    expect(rows.getByRole("toolbar", { name: "Text style" })).toBeInTheDocument()
    expect(rows.getByRole("button", { name: /^Text colour: / })).toBeInTheDocument()
    expect(rows.getByRole("combobox", { name: "Text mode" })).toHaveTextContent("Wrap")
    // The key's own values are selected.
    expect(rows.getByRole("radio", { name: "Top" })).toHaveAttribute("aria-checked", "true")
    expect(rows.getByRole("button", { name: "Bold" })).toHaveAttribute("aria-pressed", "true")
    await user.keyboard("{Escape}")
    expect(screen.queryByRole("dialog", { name: "Title appearance" })).not.toBeInTheDocument()
    // Opening and closing stored nothing.
    expect(onDirtyChange).not.toHaveBeenCalledWith(true)
  })

  it("shows the inherited defaults until the key sets its own, and Reset clears only overrides", async () => {
    const user = renderEditor()
    let rows = await open(user, "Key 1")
    // Nothing set: the device's white, bottom under the key's icon, not bold.
    expect(rows.getByRole("button", { name: /^Text colour: / })).toHaveAccessibleName(
      "Text colour: Default: #ffffff",
    )
    expect(rows.getByRole("radio", { name: "Bottom" })).toHaveAttribute("aria-checked", "true")
    expect(rows.getByRole("button", { name: "Reset title appearance" })).toBeDisabled()
    await user.keyboard("{Escape}")

    rows = await open(user, "Key 2")
    await user.click(rows.getByRole("button", { name: "Reset title appearance" }))
    const binding = await saved(user, 1)
    // Cleared, not written as the resolved white/bottom/regular.
    expect(binding?.appearance).toBeUndefined()
  })

  it("changes every part of the title, live on the Canvas, as undoable steps", async () => {
    const user = renderEditor()
    const rows = await open(user, "Key 1")
    await user.click(rows.getByRole("radio", { name: "Middle" }))
    await user.click(rows.getByRole("button", { name: "Bold" }))
    await user.click(rows.getByRole("button", { name: "Italic" }))
    await user.click(rows.getByRole("button", { name: "Underline" }))
    await user.type(rows.getByRole("spinbutton", { name: "Size (px)" }), "14")
    // The key on the Canvas is drawn again with the new look (debounced).
    await waitFor(() => {
      expect(control("Key 1").querySelector("img")?.getAttribute("src")).toContain('"bold":true')
    })
    expect(control("Key 1").querySelector("img")?.getAttribute("src")).toContain('"italic":true')
    // And the header's preview is the same image.
    expect(
      screen
        .getByRole("button", { name: "Key 1 preview: icon and background" })
        .querySelector("img"),
    ).toHaveAttribute("src", control("Key 1").querySelector("img")?.getAttribute("src") ?? "")
    expect(onDirtyChange).toHaveBeenCalledWith(true)

    // Undo takes back the size (one step for the typing), then underline.
    await user.keyboard("{Escape}")
    await user.click(screen.getByRole("button", { name: "Undo" }))
    await user.click(screen.getByRole("button", { name: "Undo" }))
    const back = await open(user, "Key 1")
    expect(back.getByRole("spinbutton", { name: "Size (px)" })).toHaveValue(null)
    expect(back.getByRole("button", { name: "Underline" })).toHaveAttribute("aria-pressed", "false")
    expect(back.getByRole("button", { name: "Italic" })).toHaveAttribute("aria-pressed", "true")
    await user.keyboard("{Escape}")
    await user.click(screen.getByRole("button", { name: "Redo" }))
    const binding = await saved(user, 0)
    expect(binding?.appearance).toEqual({
      titleAlign: "middle",
      bold: true,
      italic: true,
      underline: true,
    })
    // The title text itself is untouched.
    expect(binding?.label).toBe("Deploy")
  })

  it("hides the title keeping its text and look, and shows it again", async () => {
    const user = renderEditor()
    await open(user, "Key 2")
    await user.keyboard("{Escape}")
    // Under the title field, in the key header.
    const show = () => {
      const header = document.querySelector<HTMLElement>("[data-slot=key-appearance]")
      if (!header) throw new Error("no key header")
      return within(header).getByRole("switch", { name: "Show title" })
    }
    await user.click(show())
    expect(show()).not.toBeChecked()
    let binding = await saved(user, 1)
    expect(binding?.appearance).toEqual({
      textColor: "#ff0000",
      bold: true,
      titleAlign: "top",
      showTitle: false,
    })
    await user.click(show())
    expect(show()).toBeChecked()
    binding = await saved(user, 1)
    expect(binding?.appearance).toEqual({ textColor: "#ff0000", bold: true, titleAlign: "top" })
  })

  it("lays out the title in the four modes with only the mode's settings", async () => {
    reports = { 0: report({ mode: "autoFit", fits: false, truncated: true, fontSize: 9 }) }
    const user = renderEditor()
    const rows = await open(user, "Key 1")
    expect(rows.getByRole("spinbutton", { name: "Maximum lines" })).toBeInTheDocument()
    const mode = async (name: string) => {
      await user.click(rows.getByRole("combobox", { name: "Text mode" }))
      await user.click(within(screen.getByRole("listbox")).getByRole("option", { name }))
    }
    await mode("Single line")
    expect(rows.getByRole("radiogroup", { name: "When too long" })).toBeInTheDocument()
    expect(rows.queryByRole("spinbutton", { name: "Maximum lines" })).not.toBeInTheDocument()
    await mode("Auto fit")
    expect(rows.getByRole("spinbutton", { name: "Minimum size (pt)" })).toBeInTheDocument()
    expect(rows.getByText(/Doesn't fit at the minimum size/)).toBeInTheDocument()
    await mode("Marquee")
    expect(rows.getByRole("spinbutton", { name: "Speed (px/s)" })).toBeInTheDocument()
    // Out of range: explained, not applied.
    await user.type(rows.getByRole("spinbutton", { name: "Speed (px/s)" }), "3")
    expect(rows.getByText("Enter a whole number from 5 to 240.")).toBeInTheDocument()
    await user.click(rows.getByRole("radio", { name: "Left" }))
    const binding = await saved(user, 0)
    expect(binding?.appearance?.text).toEqual({ mode: "marquee", align: "left" })
  })

  it("is operable with the keyboard", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    const trigger = screen.getByRole("button", { name: "Title appearance" })
    trigger.focus()
    await user.keyboard("{Enter}")
    const rows = popover()
    // Focus moves into the popover; arrow keys move within the segmented
    // control and Space chooses.
    await waitFor(() => {
      expect(screen.getByRole("dialog", { name: "Title appearance" })).toContainElement(
        document.activeElement as HTMLElement,
      )
    })
    rows.getByRole("radio", { name: "Bottom" }).focus()
    await user.keyboard("{ArrowLeft}")
    expect(rows.getByRole("radio", { name: "Middle" })).toHaveFocus()
    await user.keyboard(" ")
    expect(rows.getByRole("radio", { name: "Middle" })).toHaveAttribute("aria-checked", "true")
    await user.keyboard("{Escape}")
    expect(screen.queryByRole("dialog", { name: "Title appearance" })).not.toBeInTheDocument()
    expect(trigger).toHaveFocus()
  })

  it("is the same editor for a folder key and a plugin's action, as the plugin allows", async () => {
    const user = renderEditor()
    const folder = await open(user, "Key 3")
    const parts = (rows: ReturnType<typeof popover>) =>
      [
        rows.getByRole("radiogroup", { name: "Title position" }),
        rows.getByRole("combobox", { name: "Font" }),
        rows.getByRole("toolbar", { name: "Text style" }),
        rows.getByRole("combobox", { name: "Text mode" }),
      ].map((element) => element.tagName)
    const folderParts = parts(folder)
    expect(folder.getByRole("combobox", { name: "Font" })).toBeEnabled()
    await user.keyboard("{Escape}")

    const plugin = await open(user, "Key 4")
    expect(parts(plugin)).toEqual(folderParts)
    // The plugin offers its position (suggested: top) and colour only.
    expect(plugin.getByRole("radio", { name: "Top" })).toHaveAttribute("aria-checked", "true")
    expect(plugin.getByRole("radio", { name: "Top" })).toBeEnabled()
    expect(plugin.getByRole("button", { name: /^Text colour/ })).toBeEnabled()
    expect(plugin.getByRole("combobox", { name: "Font" })).toBeDisabled()
    expect(plugin.getByRole("button", { name: "Bold" })).toBeDisabled()
    // Visibility is not offered by the plugin: the header's switch is off limits.
    expect(screen.getByRole("switch", { name: "Show title" })).toBeDisabled()
    // The user's choice beats the plugin's suggestion.
    await user.click(plugin.getByRole("radio", { name: "Bottom" }))
    const binding = await saved(user, 3)
    expect(binding?.appearance).toEqual({ titleAlign: "bottom" })

    // The folder key's title change never touches what it opens.
    const folderKey = await saved(user, 2)
    expect(folderKey?.target).toEqual({ type: "openPage", page: "tools" })
  })
})

describe("scrolling titles in the previews", { timeout: 20_000 }, () => {
  it("play on the Canvas and in the key header alike, and stand still when motion is reduced", async () => {
    reports = { 0: report({ mode: "marquee", needsScroll: true }) }
    api.previewMarquee.mockResolvedValue({
      frames: [
        { image: "data:image/png;base64,frame0", durationMs: 40 },
        { image: "data:image/png;base64,frame1", durationMs: 40 },
      ],
      cycleMs: 80,
    })
    const user = renderEditor()
    await waitFor(() => {
      expect(api.previewMarquee).toHaveBeenCalled()
    })
    await user.click(control("Key 1"))
    const header = () =>
      screen
        .getByRole("button", { name: "Key 1 preview: icon and background" })
        .querySelector("img")
    const canvas = () => control("Key 1").querySelector("img")
    await waitFor(() => {
      expect(header()).toHaveAttribute("src", "data:image/png;base64,frame1")
    })
    await waitFor(() => {
      expect(canvas()).toHaveAttribute("src", "data:image/png;base64,frame1")
    })
    const rows = await open(user, "Key 1")
    expect(rows.getByText("Scrolls on the device, as shown here.")).toBeInTheDocument()
    cleanup()

    reducedMotion = true
    api.previewMarquee.mockClear()
    const reduced = renderEditor()
    const still = await open(reduced, "Key 1")
    expect(still.getByText(/Shown still here because your system reduces motion/)).toBeVisible()
    expect(api.previewMarquee).not.toHaveBeenCalled()
    expect(header()?.getAttribute("src")).toContain("key0-")
  })
})

describe("the title appearance popover in Polish", { timeout: 20_000 }, () => {
  afterEach(async () => {
    await act(async () => {
      await i18n.changeLanguage("en")
    })
  })

  it("names its controls in Polish", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    const user = renderEditor()
    await user.click(screen.getByRole("button", { name: "Klawisz 1" }))
    await user.click(screen.getByRole("button", { name: "Wygląd tytułu" }))
    const rows = within(screen.getByRole("dialog", { name: "Wygląd tytułu" }))
    expect(rows.queryByRole("checkbox", { name: "Pokazuj tytuł" })).toBeNull()
    expect(screen.getByRole("switch", { name: "Pokazuj tytuł" })).toBeInTheDocument()
    expect(rows.getByRole("radiogroup", { name: "Położenie tytułu" })).toBeInTheDocument()
    for (const name of ["Góra", "Środek", "Dół"]) {
      expect(rows.getByRole("radio", { name })).toBeInTheDocument()
    }
    for (const name of ["Pogrubienie", "Kursywa", "Podkreślenie"]) {
      expect(rows.getByRole("button", { name })).toBeInTheDocument()
    }
    expect(rows.getByRole("combobox", { name: "Czcionka" })).toBeInTheDocument()
    expect(rows.getByRole("button", { name: "Przywróć domyślny wygląd tytułu" })).toBeDisabled()
  })
})
