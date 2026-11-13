// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { ControlBinding, DeviceLayout, DeviceModel, IconInfo, ProfileView } from "@/lib/types"

import { LayoutEditor } from "./editor"

const api = {
  previewLayout: vi.fn(),
  saveLayout: vi.fn(),
  listIcons: vi.fn(),
  importIcon: vi.fn(),
  deleteIcon: vi.fn(),
  listActionTypes: vi.fn(),
  getGestureDefaults: vi.fn(),
}

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, name: string) => api[name as keyof typeof api] }),
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}))
vi.mock("@/lib/app-context", () => ({
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

/** A fake renderer: each image names the key and its background. */
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
      face: { style: "action", title: "x" },
      image: renderImages
        ? `data:image/png;base64,key${b.control.index}-${b.appearance?.background ?? "none"}`
        : undefined,
    })),
  )
}

beforeAll(() => {
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.releasePointerCapture = () => undefined
  Element.prototype.scrollIntoView = () => undefined
})
beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset()
  api.previewLayout.mockImplementation(rendered)
  api.listIcons.mockResolvedValue([icon("rocket", "Rocket", 1), icon("star", "Star", 0)])
  api.saveLayout.mockResolvedValue(undefined)
  api.listActionTypes.mockResolvedValue([])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 300, longPressMs: 700 })
})
afterEach(cleanup)

function renderEditor() {
  const user = userEvent.setup({ applyAccept: false })
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={plus}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return user
}

const control = (name: string) => screen.getByRole("button", { name })
const panel = () => within(screen.getByRole("region", { name: "Appearance" }))

async function saved(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: /Save/ }))
  const [, layout] = api.saveLayout.mock.calls.at(-1) as [string, DeviceLayout]
  return layout.pages[0]?.bindings ?? []
}

describe("appearance editor", () => {
  it("shows rendered keys and changes colours with swatches, hex and undo", async () => {
    const user = renderEditor()
    // The canvas shows the keys as the renderer draws them.
    await waitFor(() => {
      expect(control("Key 1").querySelector("img")).toHaveAttribute(
        "src",
        "data:image/png;base64,key0-#000000",
      )
    })
    expect(api.previewLayout).toHaveBeenCalledWith("p", layout, "home", null, true)

    await user.click(control("Key 2"))
    const background = within(panel().getByRole("group", { name: "Background swatches" }))
    await user.click(background.getByRole("button", { name: "Red" }))
    expect(background.getByRole("button", { name: "Red" })).toHaveAttribute("aria-pressed", "true")
    // The rendered preview follows the edit (debounced).
    await waitFor(() => {
      expect(panel().getByRole("img", { name: "Rendered preview of Key 2" })).toHaveAttribute(
        "src",
        "data:image/png;base64,key1-#dc2626",
      )
    })

    // Typing a colour is one undo step.
    const hex = panel().getByRole("textbox", { name: "Text colour hex" })
    await user.type(hex, "#zz")
    expect(hex).toHaveAttribute("aria-invalid", "true")
    expect(panel().getByText("Enter a colour as #rrggbb.")).toBeInTheDocument()
    await user.clear(hex)
    await user.type(hex, "#dd2222")
    expect(panel().getByText(/hard to read/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(panel().getByRole("textbox", { name: "Text colour hex" })).toHaveValue("")
    expect(panel().getByText(/Contrast .*:1$/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(background.getByRole("button", { name: "Red" })).toHaveAttribute("aria-pressed", "false")
  })

  it("applies text settings to a multi-selection and skips unassigned keys", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    await user.keyboard("{Control>}")
    await user.click(control("Key 3"))
    await user.click(control("Key 6"))
    await user.keyboard("{/Control}")
    expect(panel().getByText("Applies to 2 keys")).toBeInTheDocument()
    expect(panel().getByText(/1 selected control has nothing assigned/)).toBeInTheDocument()

    await user.click(panel().getByRole("radio", { name: "Large" }))
    await user.click(panel().getByRole("radio", { name: "Top" }))
    await user.click(panel().getByRole("switch", { name: "Show title" }))
    const bindings = await saved(user)
    for (const index of [1, 2]) {
      expect(bindings.find((b) => b.control.index === index)?.appearance).toEqual({
        fontSize: "large",
        titleAlign: "top",
        showTitle: false,
      })
    }
    expect(bindings.some((b) => b.control.index === 5)).toBe(false)
  })

  it("explains when a selection cannot be styled", async () => {
    const user = renderEditor()
    await user.click(control("Dial 1"))
    expect(screen.getByText("Dials have no display to style.")).toBeInTheDocument()
    await user.click(control("Key 8"))
    expect(screen.getByText(/Assign an action, a workflow or a page first/)).toBeInTheDocument()
  })

  it("chooses icons and their fit, and deletes only unused icons", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    const icons = within(await panel().findByRole("list", { name: "Icons" }))
    expect(icons.getByRole("button", { name: "Rocket, used by 1 key" })).toHaveAttribute(
      "aria-pressed",
      "true",
    )
    expect(icons.getByRole("button", { name: "Delete Rocket" })).toBeDisabled()

    await user.click(panel().getByRole("radio", { name: "Crop" }))
    expect(panel().getByRole("img", { name: "Rocket with crop" })).toBeInTheDocument()

    await user.click(icons.getByRole("button", { name: "Star, used by 0 keys" }))
    // Moving the icon in the draft updates the counts before saving.
    expect(icons.getByRole("button", { name: "Star, used by 1 key" })).toHaveAttribute(
      "aria-pressed",
      "true",
    )
    expect(icons.getByRole("button", { name: "Delete Rocket" })).toBeEnabled()
    await user.click(icons.getByRole("button", { name: "Delete Rocket" }))
    expect(api.deleteIcon).toHaveBeenCalledWith("p", "rocket")
    await waitFor(() => {
      expect(icons.queryByRole("button", { name: /^Rocket/ })).not.toBeInTheDocument()
    })

    const bindings = await saved(user)
    expect(bindings[0]?.appearance).toEqual({
      background: "#000000",
      icon: { id: "star", fit: "crop" },
    })
  })

  it("imports PNG files and refuses other files before uploading", async () => {
    api.importIcon.mockResolvedValue(icon("new", "logo", 0))
    const user = renderEditor()
    await user.click(control("Key 2"))
    const input = await panel().findByLabelText("Icon file")

    await user.upload(input, new File(["GIF89a"], "anim.gif", { type: "image/gif" }))
    expect(panel().getByRole("alert")).toHaveTextContent("Choose a PNG or JPEG image.")
    expect(api.importIcon).not.toHaveBeenCalled()

    await user.upload(input, new File(["png-bytes"], "logo.png", { type: "image/png" }))
    await waitFor(() => {
      expect(api.importIcon).toHaveBeenCalledWith("p", "logo", btoa("png-bytes"))
    })
    // The imported icon goes onto the selected key.
    expect(await panel().findByRole("button", { name: "logo, used by 1 key" })).toHaveAttribute(
      "aria-pressed",
      "true",
    )
    expect(panel().queryByRole("alert")).not.toBeInTheDocument()
  })

  it("says which file was not imported and why", async () => {
    api.importIcon.mockRejectedValue("the image could not be decoded")
    const user = renderEditor()
    await user.click(control("Key 2"))
    const input = await panel().findByLabelText("Icon file")
    await user.upload(input, new File(["broken"], "logo.png", { type: "image/png" }))
    expect(await panel().findByRole("alert")).toHaveTextContent(
      "“logo.png” was not imported: the image could not be decoded.",
    )
    expect(input).toHaveAccessibleDescription(
      "“logo.png” was not imported: the image could not be decoded.",
    )
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
