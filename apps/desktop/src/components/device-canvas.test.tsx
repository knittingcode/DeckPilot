// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { DndContext } from "@dnd-kit/core"
import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { DeviceCanvas } from "./device-canvas"
import type { DeviceModel } from "@/lib/types"

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
    expect(onSelect).toHaveBeenCalledWith({ kind: "encoder", index: 1 })
  })
})
