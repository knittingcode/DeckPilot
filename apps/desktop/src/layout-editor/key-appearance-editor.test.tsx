// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { i18n } from "@/i18n"
import { installDomStubs } from "@/test/dom"
import type {
  ControlBinding,
  DeviceLayout,
  DeviceModel,
  IconInfo,
  ProfileView,
  VisualLibrary,
} from "@/lib/types"

import { LayoutEditor } from "./editor"
import { resetVisualLibrary } from "./use-visual-library"

const api = {
  previewLayout: vi.fn(),
  saveLayout: vi.fn(),
  listIcons: vi.fn(),
  importIcon: vi.fn(),
  deleteIcon: vi.fn(),
  pageTree: vi.fn(),
  listActionTypes: vi.fn(),
  getGestureDefaults: vi.fn(),
  titleFonts: vi.fn(),
  visualLibrary: vi.fn(),
  // What running a key would call: never from the key header.
  simulatePress: vi.fn(),
  runAction: vi.fn(),
  runWorkflow: vi.fn(),
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

const plus: DeviceModel = {
  id: "elgato.streamdeck.plus",
  vendor: "Elgato",
  name: "Stream Deck +",
  usb: null,
  capabilities: {
    keys: { rows: 2, columns: 4, image: { width: 120, height: 120, format: "jpeg" } },
    encoders: { count: 4, pressable: true },
    touch: null,
  },
  support: "unverified",
  verification: [],
}

const press = (index: number, action: string, appearance?: ControlBinding["appearance"]) =>
  ({
    control: { kind: "key", index },
    trigger: { type: "press" },
    target: { type: "action", action },
    ...(appearance ? { appearance } : {}),
  }) as ControlBinding

const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: plus.id,
  homePage: "home",
  pages: [
    {
      id: "home",
      name: "Home",
      bindings: [
        press(0, "shop", { background: "#000000", icon: { id: "rocket" } }),
        press(1, "mail"),
        press(2, "docs"),
      ],
    },
  ],
}

const icon = (id: string, name: string, usages: number): IconInfo => ({
  id,
  name,
  width: 64,
  height: 64,
  bytes: 1000,
  dataUrl: `data:image/png;base64,${id}`,
  usages,
})

const view = {
  activeProjectId: null,
  profile: {
    id: "p",
    name: "Work",
    description: "",
    actions: ["shop", "mail", "docs"].map((id) => ({ id, name: id, icon: null })),
    workflows: [],
    projects: [],
    layouts: [layout],
  },
} as unknown as ProfileView

/** A fake renderer: each image names the key and its look. */
function rendered(
  _: string,
  draft: DeviceLayout,
  pageId: string,
  __: unknown,
  renderImages?: boolean,
) {
  const page = draft.pages.find((p) => p.id === pageId)
  return Promise.resolve(
    (page?.bindings ?? []).map((b) => ({
      control: b.control,
      face: { style: "action", title: b.label ?? `${b.target.type} name` },
      image: renderImages
        ? `data:image/png;base64,key${String(b.control.index)}-${b.appearance?.background ?? "none"}-${b.appearance?.icon?.id ?? "noicon"}`
        : undefined,
    })),
  )
}

const tile = (name: string) => `data:image/png;base64,${name}`

/** DeckPilot's icons and gradients and two plugins' assets (one disabled). */
const library: VisualLibrary = {
  categories: ["media", "home"],
  icons: [
    {
      id: "deckpilot.play",
      name: "play",
      categories: ["media"],
      tags: ["start"],
      thumbnail: tile("play"),
    },
    {
      id: "deckpilot.house",
      name: "house",
      categories: ["home"],
      tags: ["home"],
      thumbnail: tile("house"),
    },
    {
      id: "deckpilot.mic",
      name: "mic",
      categories: ["media"],
      tags: ["record"],
      thumbnail: tile("mic"),
    },
  ],
  gradients: [
    {
      id: "deckpilot.gradient-ocean",
      name: "gradient-ocean",
      colors: ["#0ea5e9", "#1e3a8a"],
      thumbnail: tile("ocean"),
    },
  ],
  plugins: [
    {
      id: "acme.weather.sunny",
      plugin: "acme.weather",
      pluginName: "Weather",
      name: "Sunny",
      kind: "icon",
      keywords: ["sun"],
      available: true,
      thumbnail: tile("sunny"),
    },
    {
      id: "acme.weather.dusk",
      plugin: "acme.weather",
      pluginName: "Weather",
      name: "Dusk",
      kind: "gradientBackground",
      keywords: [],
      available: true,
      thumbnail: tile("dusk"),
    },
    {
      id: "acme.old.sky",
      plugin: "acme.old",
      pluginName: "Old",
      name: "Sky",
      kind: "imageBackground",
      keywords: [],
      available: false,
      thumbnail: tile("sky"),
    },
  ],
  version: 1,
}

/** What the WebView's clipboard holds for the next `navigator.clipboard.read()`. */
let clipboard: (() => Promise<ClipboardItems>) | null = null

beforeAll(() => {
  installDomStubs()
})
beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset()
  clipboard = null
  api.previewLayout.mockImplementation(rendered)
  api.pageTree.mockResolvedValue({ pages: [], warnings: [] })
  api.listIcons.mockResolvedValue([icon("rocket", "Rocket", 1), icon("star", "Star", 0)])
  api.saveLayout.mockImplementation((_, saved) =>
    Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
  )
  api.listActionTypes.mockResolvedValue([])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 300, longPressMs: 700 })
  api.titleFonts.mockResolvedValue([{ id: "inter", name: "Inter" }])
  api.visualLibrary.mockResolvedValue(library)
  resetVisualLibrary()
})
afterEach(cleanup)

function renderEditor(model: DeviceModel = plus) {
  const user = userEvent.setup({ applyAccept: false })
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={model}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return user
}

const control = (name: string) => screen.getByRole("button", { name })
const header = () => {
  const element = document.querySelector<HTMLElement>("[data-slot=key-appearance]")
  if (!element) throw new Error("no key header")
  return within(element)
}
const preview = (key: string) =>
  screen.getByRole("button", { name: `${key} preview: icon and background` })

async function saved(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: /Save/ }))
  const [, layout] = api.saveLayout.mock.calls.at(-1) as [string, DeviceLayout]
  return layout.pages[0]?.bindings ?? []
}

async function menu(user: ReturnType<typeof userEvent.setup>, key: string) {
  await user.click(preview(key))
  return within(await screen.findByRole("menu"))
}

describe("the key header", { timeout: 20_000 }, () => {
  it("shows the key as the Canvas draws it, and edits the visible title exactly", async () => {
    const user = renderEditor()
    await waitFor(() => {
      expect(control("Key 1").querySelector("img")).toHaveAttribute(
        "src",
        "data:image/png;base64,key0-#000000-rocket",
      )
    })
    // Drawn for the size the key is shown at: 72 CSS px less its border.
    expect(api.previewLayout).toHaveBeenCalledWith("p", layout, "home", null, true, null, 64)
    await user.click(control("Key 1"))
    // The same image as the Canvas's: one renderer, one preview.
    expect(preview("Key 1").querySelector("img")).toHaveAttribute(
      "src",
      control("Key 1").querySelector("img")?.getAttribute("src") ?? "",
    )
    const title = header().getByRole("textbox", { name: "Key title" })
    // Without a title of its own, the key's default one.
    expect(title).toHaveAttribute("placeholder", "action name")
    await user.type(title, "  Zażółć  ✓ ")
    const bindings = await saved(user)
    expect(bindings[0]?.label).toBe("  Zażółć  ✓ ")
    // Only the title changed.
    expect(bindings[0]?.target).toEqual({ type: "action", action: "shop" })
    expect(bindings[0]?.appearance).toEqual({ background: "#000000", icon: { id: "rocket" } })
  })

  it("is not shown for dials and keys without anything assigned", async () => {
    const user = renderEditor()
    await user.click(control("Dial 1"))
    expect(document.querySelector("[data-slot=key-appearance]")).toBeNull()
    await user.click(control("Key 8"))
    expect(document.querySelector("[data-slot=key-appearance]")).toBeNull()
  })

  it("never runs the key when its preview is clicked", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    await menu(user, "Key 2")
    await user.keyboard("{Escape}")
    await user.dblClick(preview("Key 2"))
    await user.keyboard("{Escape}")
    expect(api.simulatePress).not.toHaveBeenCalled()
    expect(api.runAction).not.toHaveBeenCalled()
    expect(api.runWorkflow).not.toHaveBeenCalled()
    // Still the same key selected, nothing changed.
    expect(control("Key 2")).toHaveAttribute("aria-pressed", "true")
    expect(screen.getByRole("button", { name: "Undo" })).toBeDisabled()
  })

  it("changes the background from the Background Library, as undo steps", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    await user.click(
      (await menu(user, "Key 2")).getByRole("menuitem", { name: "Choose background…" }),
    )
    const picker = within(await screen.findByRole("dialog", { name: "Background library" }))
    const solid = within(picker.getByRole("group", { name: "Solid colours" }))
    await user.click(solid.getByRole("button", { name: "Red #dc2626" }))
    expect(solid.getByRole("button", { name: "Red #dc2626" })).toHaveAttribute(
      "aria-pressed",
      "true",
    )
    await waitFor(() => {
      expect(preview("Key 2").querySelector("img")).toHaveAttribute(
        "src",
        "data:image/png;base64,key1-#dc2626-noicon",
      )
    })
    await user.keyboard("{Escape}")
    await user.click(screen.getByRole("button", { name: "Undo" }))
    // Back to the saved look: nothing left to save.
    await waitFor(() => {
      expect(preview("Key 2").querySelector("img")).toHaveAttribute(
        "src",
        "data:image/png;base64,key1-none-noicon",
      )
    })
    expect(screen.getByRole("button", { name: /Save/ })).toBeDisabled()
  })

  it("shows the device model's defaults whole and stores transparent and black as chosen", async () => {
    // A model whose display is light: white default text reads badly.
    const user = renderEditor({
      ...plus,
      defaultAppearance: {
        keyBackground: "transparent",
        textColor: "#ffffff",
        displayBackground: "#eeeeee",
      },
    })
    await user.click(control("Key 2"))
    await user.click((await menu(user, "Key 2")).getByRole("menuitem", { name: /custom colour/ }))
    const picker = within(await screen.findByRole("dialog", { name: "Choose custom colour…" }))
    expect(picker.getByText(/hard to read/)).toBeInTheDocument()
    const hex = picker.getByRole("textbox", { name: "Background hex" })
    expect(hex).toHaveAttribute("placeholder", "Default: Transparent")
    // It takes the room it has (the placeholder was cut off at a fixed width).
    expect(hex).toHaveClass("flex-1")
    await user.keyboard("{Escape}")
    await user.click(
      (await menu(user, "Key 2")).getByRole("menuitem", { name: "Choose background…" }),
    )
    let basic = within(await screen.findByRole("group", { name: "Basic" }))
    await user.click(basic.getByRole("button", { name: "Transparent" }))
    await user.keyboard("{Escape}")
    await user.click(control("Key 3"))
    await user.click(
      (await menu(user, "Key 3")).getByRole("menuitem", { name: "Choose background…" }),
    )
    basic = within(await screen.findByRole("group", { name: "Basic" }))
    await user.click(basic.getByRole("button", { name: "Black" }))
    await user.keyboard("{Escape}")
    const bindings = await saved(user)
    expect(bindings[1]?.appearance).toEqual({ background: "transparent" })
    expect(bindings[2]?.appearance).toEqual({ background: "#000000" })
  })

  it("applies the look to a multi-selection and skips unassigned keys", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    await user.keyboard("{Control>}")
    await user.click(control("Key 3"))
    await user.click(control("Key 6"))
    await user.keyboard("{/Control}")
    expect(header().getByText("Applies to 2 keys")).toBeInTheDocument()
    await user.click(header().getByRole("button", { name: "Title appearance" }))
    const rows = within(screen.getByRole("dialog", { name: "Title appearance" }))
    await user.click(rows.getByRole("radio", { name: "Top" }))
    await user.keyboard("{Escape}")
    await user.click(header().getByRole("switch", { name: "Show title" }))
    const bindings = await saved(user)
    for (const index of [1, 2]) {
      expect(bindings.find((b) => b.control.index === index)?.appearance).toEqual({
        titleAlign: "top",
        showTitle: false,
      })
    }
    expect(bindings.some((b) => b.control.index === 5)).toBe(false)
  })
})

describe("the icon menu", { timeout: 20_000 }, () => {
  it("offers only what the app can do", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    const items = (await menu(user, "Key 2"))
      .getAllByRole("menuitem")
      .map((i) => i.textContent.trim())
    expect(items).toEqual([
      "Choose from icon library…",
      "Import icon from file…",
      "Paste icon from clipboard",
      "Choose background…",
      "Choose custom colour…",
    ])
    // Two labelled sections.
    const groups = screen.getAllByRole("group").map((g) => g.getAttribute("aria-label"))
    expect(groups).toEqual(expect.arrayContaining(["Icon", "Background"]))
    await user.keyboard("{Escape}")
    // A key with an icon and a background of its own can drop them.
    await user.click(control("Key 1"))
    const own = await menu(user, "Key 1")
    expect(own.getByRole("menuitem", { name: "Remove icon" })).toBeInTheDocument()
    expect(own.getByRole("menuitem", { name: "Reset to device default" })).toBeInTheDocument()
    await user.click(own.getByRole("menuitem", { name: "Remove icon" }))
    const bindings = await saved(user)
    expect(bindings[0]?.appearance).toEqual({ background: "#000000" })
  })

  it("chooses icons and their fit from the library, and deletes only unused icons", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    await user.click((await menu(user, "Key 1")).getByRole("menuitem", { name: /icon library/ }))
    const library = within(await screen.findByRole("dialog", { name: /icon library/ }))
    // The key's icon is one of My Icons: that tab is open.
    const icons = within(await library.findByRole("group", { name: "Icons" }))
    expect(icons.getByRole("button", { name: "Rocket, used by 1 key" })).toHaveAttribute(
      "aria-pressed",
      "true",
    )
    expect(icons.getByRole("button", { name: "Delete Rocket" })).toBeDisabled()
    await user.click(library.getByRole("radio", { name: "Crop" }))
    expect(library.getByRole("img", { name: "Rocket with Crop" })).toBeInTheDocument()
    await user.click(icons.getByRole("button", { name: "Star, used by 0 keys" }))
    // Moving the icon in the draft updates the counts before saving.
    expect(icons.getByRole("button", { name: "Star, used by 1 key" })).toHaveAttribute(
      "aria-pressed",
      "true",
    )
    await user.click(icons.getByRole("button", { name: "Delete Rocket" }))
    expect(api.deleteIcon).toHaveBeenCalledWith("p", "rocket")
    await waitFor(() => {
      expect(icons.queryByRole("button", { name: /^Rocket/ })).not.toBeInTheDocument()
    })
    await user.keyboard("{Escape}")
    const bindings = await saved(user)
    expect(bindings[0]?.appearance).toEqual({
      background: "#000000",
      icon: { id: "star", fit: "crop" },
    })
  })

  it("imports PNG files through the shared checks and refuses other files", async () => {
    api.importIcon.mockResolvedValue(icon("new", "logo", 0))
    const user = renderEditor()
    await user.click(control("Key 2"))
    const input = screen.getByLabelText("Icon file")
    await user.upload(input, new File(["GIF89a"], "anim.gif", { type: "image/gif" }))
    expect(header().getByRole("alert")).toHaveTextContent("Choose a PNG or JPEG image.")
    expect(api.importIcon).not.toHaveBeenCalled()

    await user.upload(input, new File(["png-bytes"], "logo.png", { type: "image/png" }))
    await waitFor(() => {
      expect(api.importIcon).toHaveBeenCalledWith("p", "logo", btoa("png-bytes"))
    })
    // The imported icon goes onto the selected key (an undo step).
    await waitFor(() => {
      expect(preview("Key 2").querySelector("img")?.getAttribute("src")).toContain("-new")
    })
    expect(header().queryByRole("alert")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Undo" }))
    await waitFor(() => {
      expect(preview("Key 2").querySelector("img")?.getAttribute("src")).toContain("-noicon")
    })
  })

  it("says which file was not imported and why", async () => {
    api.importIcon.mockRejectedValue("the image could not be decoded")
    const user = renderEditor()
    await user.click(control("Key 2"))
    await user.upload(
      screen.getByLabelText("Icon file"),
      new File(["broken"], "logo.png", { type: "image/png" }),
    )
    expect(await header().findByRole("alert")).toHaveTextContent(
      "“logo.png” was not imported: the image could not be decoded.",
    )
  })

  it("pastes a PNG from the clipboard, and explains what cannot be pasted", async () => {
    api.importIcon.mockResolvedValue(icon("pasted", "Pasted icon", 0))
    const user = renderEditor()
    // After `userEvent.setup()`, which puts a clipboard of its own in place.
    vi.spyOn(navigator.clipboard, "read").mockImplementation(() =>
      clipboard ? clipboard() : Promise.resolve([]),
    )
    await user.click(control("Key 2"))
    const item = (types: string[]) =>
      ({
        types,
        getType: (type: string) => Promise.resolve(new Blob(["png-bytes"], { type })),
      }) as unknown as ClipboardItem

    // An image of another format: refused before uploading.
    clipboard = () => Promise.resolve([item(["image/gif"])])
    await user.click((await menu(user, "Key 2")).getByRole("menuitem", { name: /Paste/ }))
    expect(await header().findByRole("alert")).toHaveTextContent(
      "The clipboard image is not PNG or JPEG.",
    )
    // Text only.
    clipboard = () => Promise.resolve([item(["text/plain"])])
    await user.click((await menu(user, "Key 2")).getByRole("menuitem", { name: /Paste/ }))
    expect(await header().findByText("The clipboard has no image.")).toBeInTheDocument()
    // Not allowed by the system.
    clipboard = () => Promise.reject(new Error("NotAllowedError"))
    await user.click((await menu(user, "Key 2")).getByRole("menuitem", { name: /Paste/ }))
    expect(await header().findByText(/may not read the clipboard/)).toBeInTheDocument()
    expect(api.importIcon).not.toHaveBeenCalled()

    // A PNG: imported like a file and put on the key.
    clipboard = () => Promise.resolve([item(["text/html", "image/png"])])
    await user.click((await menu(user, "Key 2")).getByRole("menuitem", { name: /Paste/ }))
    await waitFor(() => {
      expect(api.importIcon).toHaveBeenCalledWith("p", "Pasted icon", btoa("png-bytes"))
    })
    await waitFor(() => {
      expect(header().queryByRole("alert")).not.toBeInTheDocument()
    })
    const bindings = await saved(user)
    expect(bindings[1]?.appearance).toEqual({ icon: { id: "pasted" } })
  })

  it("counts keys with the Polish plural forms", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    try {
      const user = renderEditor()
      const byId = (id: string) => document.querySelector(`[data-control="${id}"]`) as HTMLElement
      await user.click(byId("key:1"))
      await user.keyboard("{Control>}")
      await user.click(byId("key:2"))
      await user.keyboard("{/Control}")
      expect(header().getByText("Dotyczy 2 klawiszy")).toBeInTheDocument()
      await user.click(header().getByRole("button", { name: "Ikona i tło" }))
      const menuItems = within(await screen.findByRole("menu"))
      await user.click(menuItems.getByRole("menuitem", { name: /biblioteki ikon/ }))
      // Mixed icons: the DeckPilot tab first; My Icons on request.
      await user.click(await screen.findByRole("radio", { name: "Moje ikony" }))
      const icons = within(await screen.findByRole("group", { name: "Ikony" }))
      // 0: many; 1: one.
      expect(
        icons.getByRole("button", { name: "Star, używana przez 0 klawiszy" }),
      ).toBeInTheDocument()
      // Rocket on the two selected keys too: 3, few.
      await user.click(icons.getByRole("button", { name: "Rocket, używana przez 1 klawisz" }))
      expect(
        icons.getByRole("button", { name: "Rocket, używana przez 3 klawisze" }),
      ).toBeInTheDocument()
    } finally {
      await act(async () => {
        await i18n.changeLanguage("en")
      })
    }
  })

  it("carries the appearance when copying keys", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    await user.keyboard("{Control>}c{/Control}")
    await user.click(control("Key 5"))
    await user.keyboard("{Control>}v{/Control}")
    const bindings = await saved(user)
    expect(bindings.find((b) => b.control.index === 4)?.appearance).toEqual({
      background: "#000000",
      icon: { id: "rocket" },
    })
  })
})

describe("the shared header layout", { timeout: 20_000 }, () => {
  it("has the title with T on the first row and Show title under it, no Background row", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    const element = document.querySelector<HTMLElement>("[data-slot=key-appearance]")
    expect(element?.querySelector("[data-slot=key-background]")).toBeNull()
    expect(header().queryByText("Background")).toBeNull()
    const title = header().getByRole("textbox", { name: "Key title" })
    const show = header().getByRole("switch", { name: "Show title" })
    // Show title follows the title row, in the same column.
    expect(title.compareDocumentPosition(show) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(show.closest("[data-slot=show-title]")?.parentElement).toBe(
      title.closest("[data-slot=key-title]")?.parentElement?.parentElement,
    )
    // Hiding keeps the title text; showing again restores it.
    await user.type(title, "Shop")
    await user.click(show)
    expect(title).toHaveValue("Shop")
    const bindings = await saved(user)
    expect(bindings[1]?.label).toBe("Shop")
    expect(bindings[1]?.appearance).toEqual({ showTitle: false })
  })
})

describe("the Icon Library", { timeout: 20_000 }, () => {
  async function openLibrary(user: ReturnType<typeof userEvent.setup>, key: string) {
    await user.click(control(key))
    await user.click((await menu(user, key)).getByRole("menuitem", { name: /icon library/ }))
    return within(await screen.findByRole("dialog", { name: /icon library/ }))
  }

  it("is populated with DeckPilot's icons, searched and filtered by category", async () => {
    const user = renderEditor()
    const dialog = await openLibrary(user, "Key 2")
    const grid = () => within(dialog.getByRole("group", { name: "DeckPilot icons" }))
    expect(grid().getByRole("button", { name: "play" })).toBeInTheDocument()
    expect(grid().getByRole("button", { name: "house" })).toBeInTheDocument()
    // By a search word, not only the name.
    await user.type(dialog.getByRole("searchbox", { name: "Search icons" }), "home")
    expect(grid().queryByRole("button", { name: "play" })).toBeNull()
    expect(grid().getByRole("button", { name: "house" })).toBeInTheDocument()
    await user.clear(dialog.getByRole("searchbox", { name: "Search icons" }))
    await user.selectOptions(dialog.getByRole("combobox", { name: "Category" }), "media")
    expect(grid().queryByRole("button", { name: "house" })).toBeNull()
    expect(
      grid()
        .getAllByRole("button")
        .map((b) => b.getAttribute("aria-label")),
    ).toEqual(["play", "mic"])
  })

  it("puts the chosen icon on the key through the draft, and runs nothing", async () => {
    const user = renderEditor()
    const dialog = await openLibrary(user, "Key 2")
    await user.click(dialog.getByRole("button", { name: "house" }))
    expect(dialog.getByRole("button", { name: "house" })).toHaveAttribute("aria-pressed", "true")
    await waitFor(() => {
      expect(preview("Key 2").querySelector("img")?.getAttribute("src")).toContain(
        "-deckpilot.house",
      )
    })
    expect(api.simulatePress).not.toHaveBeenCalled()
    expect(api.runAction).not.toHaveBeenCalled()
    await user.keyboard("{Escape}")
    // Focus goes back to the preview.
    expect(preview("Key 2")).toHaveFocus()
    await user.click(screen.getByRole("button", { name: "Undo" }))
    await waitFor(() => {
      expect(preview("Key 2").querySelector("img")?.getAttribute("src")).toContain("-noicon")
    })
    await user.click(screen.getByRole("button", { name: "Redo" }))
    const bindings = await saved(user)
    expect(bindings[1]?.appearance).toEqual({ icon: { id: "deckpilot.house" } })
  })

  it("moves between tiles with the arrow keys", async () => {
    const user = renderEditor()
    const dialog = await openLibrary(user, "Key 2")
    const grid = within(dialog.getByRole("group", { name: "DeckPilot icons" }))
    const none = grid.getByRole("button", { name: "No icon" })
    expect(none).toHaveAttribute("tabindex", "0")
    none.focus()
    await user.keyboard("{ArrowRight}")
    expect(grid.getByRole("button", { name: "play" })).toHaveFocus()
    await user.keyboard("{End}")
    expect(grid.getByRole("button", { name: "mic" })).toHaveFocus()
    await user.keyboard("{Home}")
    expect(none).toHaveFocus()
    await user.keyboard("{Enter}")
    expect(none).toHaveAttribute("aria-pressed", "true")
  })

  it("lists the plugins' icons, a disabled plugin's ones not choosable", async () => {
    const user = renderEditor()
    const dialog = await openLibrary(user, "Key 2")
    await user.click(dialog.getByRole("radio", { name: "Plugins" }))
    const grid = within(dialog.getByRole("group", { name: "Plugin icons" }))
    await user.click(grid.getByRole("button", { name: "Sunny (Weather)" }))
    await user.keyboard("{Escape}")
    const bindings = await saved(user)
    expect(bindings[1]?.appearance).toEqual({ icon: { id: "acme.weather.sunny" } })
  })
})

describe("the Background Library", { timeout: 20_000 }, () => {
  async function openBackgrounds(user: ReturnType<typeof userEvent.setup>, key: string) {
    await user.click(control(key))
    await user.click((await menu(user, key)).getByRole("menuitem", { name: "Choose background…" }))
    return within(await screen.findByRole("dialog", { name: "Background library" }))
  }

  it("offers Transparent, Black and White first, then colours, gradients, images and plugins", async () => {
    const user = renderEditor()
    const dialog = await openBackgrounds(user, "Key 2")
    const sections = dialog.getAllByRole("region").map((r) => r.getAttribute("aria-label"))
    expect(sections).toEqual([
      "Basic",
      "Solid colours",
      "Gradients",
      "Images",
      "Plugin backgrounds",
    ])
    const basic = within(dialog.getByRole("group", { name: "Basic" }))
    expect(basic.getAllByRole("button").map((b) => b.getAttribute("aria-label"))).toEqual([
      "Transparent",
      "Black",
      "White",
    ])
    // Obvious basics carry no visible label.
    expect(basic.queryByText("Black")).toBeNull()
    const plugins = within(dialog.getByRole("group", { name: "Plugin backgrounds" }))
    expect(plugins.getByRole("button", { name: "Sky (Old, disabled)" })).toBeDisabled()
    await user.click(plugins.getByRole("button", { name: "Dusk (Weather)" }))
    await user.keyboard("{Escape}")
    let bindings = await saved(user)
    expect(bindings[1]?.appearance).toEqual({ background: "asset:acme.weather.dusk" })
    // A gradient, then an image of the profile.
    const again = await openBackgrounds(user, "Key 3")
    await user.click(
      within(again.getByRole("group", { name: "Gradients" })).getByRole("button", {
        name: "Ocean",
      }),
    )
    await user.click(
      within(again.getByRole("group", { name: "Images" })).getByRole("button", { name: "Star" }),
    )
    await user.keyboard("{Escape}")
    bindings = await saved(user)
    expect(bindings[2]?.appearance).toEqual({ background: "asset:star" })
  })

  it("takes any custom colour, yellow included, and resets to the device default", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    await user.click((await menu(user, "Key 2")).getByRole("menuitem", { name: /custom colour/ }))
    const dialog = within(await screen.findByRole("dialog", { name: "Choose custom colour…" }))
    await user.type(dialog.getByRole("textbox", { name: "Background hex" }), "#ffd400")
    await user.keyboard("{Escape}")
    const bindings = await saved(user)
    expect(bindings[1]?.appearance).toEqual({ background: "#ffd400" })
    // Inherit again: nothing stored, the key as saved before.
    await user.click(
      (await menu(user, "Key 2")).getByRole("menuitem", { name: "Reset to device default" }),
    )
    await waitFor(() => {
      expect(preview("Key 2").querySelector("img")?.getAttribute("src")).toContain("key1-none")
    })
    expect(screen.getByRole("button", { name: /Save/ })).toBeDisabled()
  })

  it("says when a key's background is unavailable and keeps it", async () => {
    const stale: DeviceLayout = {
      ...layout,
      pages: [
        {
          id: "home",
          name: "Home",
          bindings: [press(0, "shop", { background: "asset:acme.old.sky" }), press(1, "mail")],
        },
      ],
    }
    const user = userEvent.setup({ applyAccept: false })
    render(
      <LayoutEditor
        profileId="p"
        view={{ ...view, profile: { ...view.profile, layouts: [stale] } }}
        model={plus}
        saved={stale}
        onDirtyChange={vi.fn()}
        onSaved={vi.fn()}
      />,
    )
    await user.click(control("Key 1"))
    await user.click(
      (await menu(user, "Key 1")).getByRole("menuitem", { name: "Choose background…" }),
    )
    const dialog = within(await screen.findByRole("dialog", { name: "Background library" }))
    expect(dialog.getByRole("status")).toHaveTextContent(/disabled plugin/)
    await user.keyboard("{Escape}")
    // Nothing rewrote it: still the same reference, nothing to save.
    expect(screen.getByRole("button", { name: /Save/ })).toBeDisabled()
  })

  it("names the menu sections in Polish", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    try {
      const user = renderEditor()
      await user.click(screen.getByRole("button", { name: "Klawisz 2" }))
      await user.click(screen.getByRole("button", { name: "Podgląd: Klawisz 2, ikona i tło" }))
      const items = within(await screen.findByRole("menu"))
      expect(items.getByText("Ikona")).toBeInTheDocument()
      expect(items.getByText("Tło")).toBeInTheDocument()
      await user.click(items.getByRole("menuitem", { name: "Wybierz tło…" }))
      const dialog = within(await screen.findByRole("dialog", { name: "Biblioteka teł" }))
      expect(dialog.getByRole("group", { name: "Podstawowe" })).toBeInTheDocument()
      expect(dialog.getByRole("button", { name: "Przezroczysty" })).toBeInTheDocument()
    } finally {
      await act(async () => {
        await i18n.changeLanguage("en")
      })
    }
  })
})
