import { vi } from "vitest"

/**
 * Browser APIs jsdom lacks but Radix (selects, menus, tooltips, dialogs)
 * and the sidebar rely on. Call once per test file, e.g. in `beforeAll`.
 */
export function installDomStubs() {
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
  if (typeof window.matchMedia !== "function") {
    window.matchMedia = vi.fn().mockReturnValue({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })
  }
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.releasePointerCapture = () => undefined
  Element.prototype.scrollIntoView = () => undefined
}
