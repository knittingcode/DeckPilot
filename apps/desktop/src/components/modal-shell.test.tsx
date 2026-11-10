// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { ModalShell } from "./modal-shell"

afterEach(cleanup)

describe("ModalShell in a native window", () => {
  it("focuses the field marked autofocus and labels the dialog", () => {
    render(
      <ModalShell title="Unlock" description="Enter your password" blocking>
        <input aria-label="Other" />
        <input aria-label="Password" data-autofocus />
      </ModalShell>,
    )
    expect(screen.getByLabelText("Password")).toHaveFocus()
    const dialog = screen.getByRole("alertdialog", { name: "Unlock" })
    expect(dialog).toHaveAttribute("aria-modal", "true")
    expect(dialog).toHaveAccessibleDescription("Enter your password")
  })

  it("closes on Escape unless it is blocking", () => {
    const onClose = vi.fn()
    const { rerender } = render(<ModalShell title="Summary" onClose={onClose} />)
    fireEvent.keyDown(window, { key: "Escape" })
    expect(onClose).toHaveBeenCalledTimes(1)
    rerender(<ModalShell title="Summary" onClose={onClose} blocking />)
    fireEvent.keyDown(window, { key: "Escape" })
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it("submits with Enter and shows loading and errors", async () => {
    const user = userEvent.setup()
    const onSubmit = vi.fn()
    const { rerender } = render(
      <ModalShell title="Form" onSubmit={onSubmit} primary={{ label: "Save" }}>
        <input aria-label="Name" />
      </ModalShell>,
    )
    await user.type(screen.getByLabelText("Name"), "x{Enter}")
    expect(onSubmit).toHaveBeenCalledTimes(1)
    rerender(
      <ModalShell
        title="Form"
        onSubmit={onSubmit}
        error="It failed"
        primary={{ label: "Save", loading: true, loadingLabel: "Saving…" }}
      >
        <input aria-label="Name" />
      </ModalShell>,
    )
    expect(screen.getByRole("alert")).toHaveTextContent("It failed")
    expect(screen.getByRole("button", { name: /saving/i })).toBeDisabled()
    rerender(<ModalShell title="Form" loading />)
    expect(screen.getByRole("status")).toHaveTextContent("Loading")
  })

  it("lays out at its natural size and keeps the footer reachable", () => {
    render(
      <ModalShell title="Logs" primary={{ label: "Close" }}>
        <pre>{"line\n".repeat(500)}</pre>
      </ModalShell>,
    )
    const root = document.querySelector<HTMLElement>("[data-modal-root]")
    const body = document.querySelector<HTMLElement>('[data-modal-part="body"]')
    const footer = document.querySelector<HTMLElement>('[data-modal-part="footer"]')
    // Nothing depends on the window's height: no viewport-height limits and
    // no inner scroll container, so the measured size is the natural one.
    for (const element of [root, body]) {
      expect(element?.className).not.toMatch(/max-h-|h-svh|overflow-/)
    }
    // When the document has to scroll, the actions stay visible.
    expect(footer?.className).toContain("sticky")
    expect(footer?.className).toContain("bottom-0")
  })
})

describe("ModalShell as an in-app dialog", () => {
  it("can be dismissed unless it is blocking", async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    const { rerender } = render(
      <ModalShell presentation="dialog" title="Details" onClose={onClose}>
        <p>Content</p>
      </ModalShell>,
    )
    await user.keyboard("{Escape}")
    expect(onClose).toHaveBeenCalledTimes(1)

    rerender(
      <ModalShell presentation="dialog" title="Confirm password" onClose={onClose} blocking>
        <p>Content</p>
      </ModalShell>,
    )
    await user.keyboard("{Escape}")
    expect(onClose).toHaveBeenCalledTimes(1)
    expect(screen.queryByRole("button", { name: /close/i })).not.toBeInTheDocument()
    expect(screen.getByText("Confirm password")).toBeInTheDocument()
  })
})
