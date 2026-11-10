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
 * size changed by at least a pixel. Returns `schedule` and `dispose`.
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
  let last: Size | null = null
  let disposed = false
  const flush = () => {
    pending = false
    if (disposed) return
    const size = measure()
    if (!size || size.width <= 0 || size.height <= 0) return
    if (last && Math.abs(last.width - size.width) < 1 && Math.abs(last.height - size.height) < 1) {
      return
    }
    // Only the content size decides whether to report: the viewport changes
    // as a result of the report, which must not trigger another one.
    last = { width: size.width, height: size.height }
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
