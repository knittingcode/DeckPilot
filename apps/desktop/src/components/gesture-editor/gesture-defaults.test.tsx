// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { api, resetApi } from "@/test/api-mock"

import { GestureDefaultsCard } from "./gesture-defaults"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeEach(() => {
  resetApi()
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 300, longPressMs: 700 })
  api.setGestureDefaults.mockImplementation((timings) => Promise.resolve(timings))
})
afterEach(cleanup)

describe("Settings › Devices gesture timing", () => {
  it("shows the stored defaults and saves valid changes", async () => {
    const user = userEvent.setup()
    render(<GestureDefaultsCard />)
    const double = await screen.findByLabelText("Double-click window (ms)")
    await waitFor(() => {
      expect(double).toHaveValue(300)
    })
    const long = screen.getByLabelText("Long-press time (ms)")
    expect(long).toHaveValue(700)
    const save = screen.getByRole("button", { name: "Save" })
    expect(save).toBeDisabled()
    expect(screen.getByRole("button", { name: "Restore defaults" })).toBeDisabled()

    await user.clear(long)
    await user.type(long, "6000")
    expect(long).toHaveAttribute("aria-invalid", "true")
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Long-press time must be between 300 and 5000 ms.",
    )
    expect(save).toBeDisabled()

    await user.clear(long)
    await user.type(long, "1200")
    await user.clear(double)
    await user.type(double, "450")
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
    await user.click(save)
    expect(api.setGestureDefaults).toHaveBeenCalledWith({ doubleClickMs: 450, longPressMs: 1200 })
    await waitFor(() => {
      expect(save).toBeDisabled()
    })

    await user.click(screen.getByRole("button", { name: "Restore defaults" }))
    expect(api.setGestureDefaults).toHaveBeenLastCalledWith({
      doubleClickMs: 300,
      longPressMs: 700,
    })
    await waitFor(() => {
      expect(long).toHaveValue(700)
    })
  })

  it("shows what the backend refused", async () => {
    api.setGestureDefaults.mockRejectedValue(
      new Error("the double-click window must be between 150 and 1000 ms, not 2000 ms"),
    )
    const user = userEvent.setup()
    render(<GestureDefaultsCard />)
    const double = await screen.findByLabelText("Double-click window (ms)")
    await waitFor(() => {
      expect(double).toHaveValue(300)
    })
    await user.clear(double)
    await user.type(double, "500")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(await screen.findByText(/not 2000 ms/)).toBeInTheDocument()
  })
})
