/**
 * Sizing native windows to their content. The window shell marks its parts
 * (`data-modal-root`, `data-modal-part`); the natural height is the header,
 * the full (unscrolled) body and the footer, so content that is taller than
 * the screen allowance scrolls inside the body instead of growing further.
 */

/**
 * The natural size of a window shell: its border box, which is laid out
 * independently of the window's height (see `ModalShell`). Rounded up so
 * that sub-pixel layout never clips the last line.
 */
export function naturalSize(root: HTMLElement): { width: number; height: number } {
  const box = root.getBoundingClientRect()
  return {
    width: Math.ceil(Math.max(box.width, root.scrollWidth)),
    height: Math.ceil(Math.max(box.height, root.scrollHeight)),
  }
}

export interface Size {
  width: number
  height: number
}

/** A measurement: the content's natural size and the webview's viewport. */
export interface Measurement extends Size {
  viewportWidth: number
  viewportHeight: number
}

/**
 * Coalesces size reports: at most one per `delay` ms, and only when the
 * measurement changed by at least a pixel in any of its values: the
 * content size, and the viewport where the measurement has one.
 *
 * The viewport counts because the native side derives the window's
 * decorations (the macOS title bar) from window size minus viewport. A
 * viewport measured while the window was still being resized gives a
 * wrong difference, and the window ends up taller than its content; the
 * report after the viewport settles corrects it. This cannot loop: a fit
 * that keeps the window's size does not resize it (`windows::fit` skips
 * changes below a pixel), so the viewport stays and nothing is reported.
 *
 * Timers, not animation frames: windows stay hidden until they are sized,
 * and WebKit does not run animation frames for hidden windows.
 */
export function coalescedReporter<M extends Size>(
  measure: () => M | null,
  report: (measurement: M) => void,
  delay = 40,
): { schedule: () => void; dispose: () => void } {
  let pending = false
  let timer: ReturnType<typeof setTimeout> | undefined
  let last: M | null = null
  let disposed = false
  const same = (a: M, b: M) =>
    (Object.keys(b) as (keyof M)[]).every((key) => {
      const x = a[key]
      const y = b[key]
      return typeof x === "number" && typeof y === "number" ? Math.abs(x - y) < 1 : x === y
    })
  const flush = () => {
    pending = false
    if (disposed) return
    const size = measure()
    if (!size || size.width <= 0 || size.height <= 0) return
    if (last && same(last, size)) return
    last = size
    report(size)
  }
  return {
    schedule: () => {
      if (pending || disposed) return
      pending = true
      timer = setTimeout(flush, delay)
    },
    dispose: () => {
      disposed = true
      if (timer) clearTimeout(timer)
    },
  }
}
