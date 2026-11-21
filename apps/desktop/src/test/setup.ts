/**
 * Global test setup (vite.config.ts `test.setupFiles`): browser APIs that
 * neither Node nor jsdom has, which Radix positioning (tooltips, popovers,
 * menus) uses whenever one opens, so that no test depends on another
 * having stubbed them and nothing fails after a test ends.
 */
if (!("ResizeObserver" in globalThis)) {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  Object.defineProperty(globalThis, "ResizeObserver", {
    value: ResizeObserverStub,
    configurable: true,
    writable: true,
  })
}
