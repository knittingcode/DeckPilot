// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { DndContext } from "@dnd-kit/core"
import { act, cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { DeviceCanvas } from "./device-canvas"
import { i18n } from "@/i18n"
import type { ControlId, DeviceModel } from "@/lib/types"
import { DEVICE_MODELS } from "@/test/fixtures"

const base: DeviceModel = {
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
}

const xl: DeviceModel = {
  ...base,
  id: "elgato.streamdeck.xl",
  name: "Stream Deck XL",
  capabilities: { ...base.capabilities, keys: { rows: 4, columns: 8, image: null } },
}

const plus: DeviceModel = {
  ...base,
  id: "elgato.streamdeck.plus",
  name: "Stream Deck +",
  capabilities: {
    keys: { rows: 2, columns: 4, image: null },
    encoders: { count: 4, pressable: true },
    touch: { zones: 4, image: null, gestures: ["tap"] },
  },
}

afterEach(cleanup)

function renderCanvas(model: DeviceModel, onSelect = vi.fn()) {
  render(
    <DndContext>
      <DeviceCanvas
        model={model}
        onSelect={onSelect}
        faces={new Map([["key:0", { style: "action", title: "SHOP" }]])}
        droppable
      />
    </DndContext>,
  )
  return onSelect
}

describe("DeviceCanvas", () => {
  it("draws every control from the model's capabilities", () => {
    renderCanvas(base)
    expect(screen.getAllByRole("button", { name: /^Key \d+$/ })).toHaveLength(15)
    cleanup()
    renderCanvas(xl)
    expect(screen.getAllByRole("button", { name: /^Key \d+$/ })).toHaveLength(32)
    cleanup()
    renderCanvas(plus)
    expect(screen.getAllByRole("button", { name: /^Key \d+$/ })).toHaveLength(8)
    expect(screen.getAllByRole("button", { name: /^Dial \d+$/ })).toHaveLength(4)
    expect(screen.getAllByRole("button", { name: /^Touch zone \d+$/ })).toHaveLength(4)
  })

  it("shows key faces and reports the selected control", async () => {
    const user = userEvent.setup()
    const onSelect = renderCanvas(plus)
    expect(screen.getByText("SHOP")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Dial 2" }))
    expect(onSelect).toHaveBeenCalledWith({ kind: "encoder", index: 1 }, "only")
  })

  it("reports selection modes and moves between rows with arrow keys", async () => {
    const user = userEvent.setup()
    const onSelect = renderCanvas(plus)
    await user.keyboard("{Shift>}")
    await user.click(screen.getByRole("button", { name: "Key 2" }))
    await user.keyboard("{/Shift}")
    expect(onSelect).toHaveBeenLastCalledWith({ kind: "key", index: 1 }, "extend")
    await user.keyboard("{Control>}")
    await user.click(screen.getByRole("button", { name: "Key 6" }))
    await user.keyboard("{/Control}")
    expect(onSelect).toHaveBeenLastCalledWith({ kind: "key", index: 5 }, "toggle")
    await user.keyboard("{ArrowDown}")
    expect(onSelect).toHaveBeenLastCalledWith({ kind: "touchZone", index: 1 }, "only")
    expect(screen.getByRole("button", { name: "Touch zone 2" })).toHaveFocus()
    await user.keyboard("{Shift>}{ArrowDown}{/Shift}")
    expect(onSelect).toHaveBeenLastCalledWith({ kind: "encoder", index: 1 }, "extend")
  })

  it("keeps a single tab stop", () => {
    renderCanvas(base)
    const keys = screen.getAllByRole("button", { name: /^Key \d+$/ })
    expect(keys.filter((k) => k.tabIndex === 0)).toHaveLength(1)
  })

  it("lays out every model from its descriptor", () => {
    const columns = () =>
      (document.querySelector("[data-slot=device-keys]") as HTMLElement).style.gridTemplateColumns
    for (const [model, cols, keys] of [
      [DEVICE_MODELS.mk2, 5, 15],
      [DEVICE_MODELS.mini, 3, 6],
      [DEVICE_MODELS.xl, 8, 32],
    ] as const) {
      renderCanvas(model)
      expect(columns()).toBe(`repeat(${cols}, var(--key))`)
      expect(screen.getAllByRole("button", { name: /^Key \d+$/ })).toHaveLength(keys)
      expect(document.querySelector("[data-slot=device-strip]")).toBeNull()
      expect(document.querySelector("[data-slot=device-dials]")).toBeNull()
      cleanup()
    }

    renderCanvas(DEVICE_MODELS.plus)
    expect(columns()).toBe("repeat(4, var(--key))")
    const strip = document.querySelector<HTMLElement>("[data-slot=device-strip]")
    const dials = document.querySelector<HTMLElement>("[data-slot=device-dials]")
    // The strip and the dials span the four columns of keys.
    expect(strip?.style.width).toBe("calc(var(--key) * 4.6)")
    expect(dials?.style.width).toBe("calc(var(--key) * 4.6)")
    expect(strip?.querySelectorAll("[data-shape=zone]")).toHaveLength(4)
    expect(dials?.querySelectorAll("[data-shape=dial]")).toHaveLength(4)
  })

  it("scales everything from the size of one key", () => {
    render(
      <DndContext>
        <DeviceCanvas model={DEVICE_MODELS.mk2} keySize={88} />
      </DndContext>,
    )
    const canvas = screen.getByRole("group", { name: "Stream Deck MK.2 controls" })
    expect(canvas.style.getPropertyValue("--key")).toBe("88px")
    // In whole pixels, so every key (and its image) starts on a pixel
    // boundary and is not resampled: 0.08 keys of room for the selection
    // ring (no enclosure padding) and 0.2 keys of gap, rounded.
    expect(canvas.style.padding).toBe("7px")
    expect(canvas.className).not.toMatch(/\b(bg-|border|rounded|shadow)/)
    expect(document.querySelector<HTMLElement>("[data-slot=device-keys]")?.style.gap).toBe("18px")
    cleanup()
    // Without a size: the size step, as before.
    render(
      <DndContext>
        <DeviceCanvas model={DEVICE_MODELS.mk2} size="sm" />
      </DndContext>,
    )
    expect(
      screen
        .getByRole("group", { name: "Stream Deck MK.2 controls" })
        .style.getPropertyValue("--key"),
    ).toBe("2.25rem")
  })

  it("marks selection, focus and drops without changing the size of a key", () => {
    const key = (selection: ControlId[]) => {
      render(
        <DndContext>
          <DeviceCanvas model={DEVICE_MODELS.mk2} selection={selection} onSelect={vi.fn()} />
        </DndContext>,
      )
      const classes = screen.getByRole("button", { name: "Key 1" }).className.split(" ")
      cleanup()
      return classes
    }
    const plain = key([])
    const selected = key([{ kind: "key", index: 0 }])
    // One border in every state; selection changes only its colour.
    expect(plain).toContain("border-(length:--key-border)")
    expect(selected).toContain("border-(length:--key-border)")
    expect(plain).toContain("border-device-key-border")
    expect(selected).toContain("border-device-selected")
    expect(selected).not.toContain("border-device-key-border")
    expect(plain).toContain("focus-visible:outline-device-focus")
    // Everything that sizes the key is the same.
    const sizing = (classes: string[]) =>
      classes.filter((c) =>
        /^(border(-\d|-x|-y|-t|-b|-l|-r)?$|border-\(length|w-|h-|size-|aspect-|p-|m-|rounded)/.test(
          c,
        ),
      )
    expect(sizing(selected)).toEqual(sizing(plain))
    expect(sizing(plain)).toContain("aspect-square")
    // What differs is colour only: no ring, glow, scale or wider border.
    const added = selected.filter((c) => !plain.includes(c))
    expect(added).toEqual(["border-device-selected"])
    expect([...plain, ...selected].some((c) => /^(ring|shadow|scale|border-[2-8]$)/.test(c))).toBe(
      false,
    )
  })

  it("scales the border with the key, in whole pixels", () => {
    for (const [size, border] of [
      [102, "6px"],
      [88, "5px"],
      [72, "4px"],
      [24, "2px"],
    ] as const) {
      render(
        <DndContext>
          <DeviceCanvas model={DEVICE_MODELS.plus} keySize={size} />
        </DndContext>,
      )
      const canvas = screen.getByRole("group", { name: "Stream Deck + controls" })
      expect(canvas.style.getPropertyValue("--key-border")).toBe(border)
      cleanup()
    }
  })

  it("draws the device's key image edge to edge, and neutral empty keys", () => {
    render(
      <DndContext>
        <DeviceCanvas
          model={DEVICE_MODELS.mk2}
          images={new Map([["key:0", "data:image/png;base64,AAAA"]])}
          faces={new Map([["key:1", { style: "empty", title: "" }]])}
        />
      </DndContext>,
    )
    const image = screen.getByRole("button", { name: "Key 1" }).querySelector("img")
    expect(image).toHaveClass("size-full", "object-cover")
    expect(image?.style.imageRendering).toBe("")
    const empty = screen.getByRole("button", { name: "Key 2" })
    expect(empty.querySelector("[data-face=empty]")).toBeInTheDocument()
    expect(empty).toHaveTextContent("")
  })
})

describe("DeviceCanvas in Polish", () => {
  afterEach(async () => {
    await act(async () => {
      await i18n.changeLanguage("en")
    })
  })

  it("names the controls in the window's language and follows a switch", async () => {
    renderCanvas(plus)
    expect(screen.getByRole("button", { name: "Key 1" })).toHaveAttribute("data-control", "key:0")
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    expect(screen.getAllByRole("button", { name: /^Klawisz \d+$/ })).toHaveLength(8)
    expect(screen.getAllByRole("button", { name: /^Pokrętło \d+$/ })).toHaveLength(4)
    expect(screen.getAllByRole("button", { name: /^Strefa dotykowa \d+$/ })).toHaveLength(4)
    const dial = screen.getByRole("button", { name: "Pokrętło 2" })
    expect(dial).toHaveAttribute("aria-label", "Pokrętło 2")
    expect(dial).toHaveAttribute("data-control", "encoder:1")
    expect(screen.queryByRole("button", { name: /^Key \d+$/ })).not.toBeInTheDocument()
  })
})
