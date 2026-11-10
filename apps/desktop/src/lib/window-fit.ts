/**
 * Sizing native windows to their content. The window shell marks its parts
 * (`data-modal-root`, `data-modal-part`); the natural height is the header,
 * the full (unscrolled) body and the footer, so content that is taller than
 * the screen allowance scrolls inside the body instead of growing further.
 */

/** The natural size of a window shell's content, for sizing its window. */
export function naturalSize(root: HTMLElement): { width: number; height: number } {
  const part = (name: string) => root.querySelector<HTMLElement>(`[data-modal-part="${name}"]`)
  const header = part("header")
  const body = part("body")
  const footer = part("footer")
  const spacer = footer ? 0 : 16
  const height =
    (header?.offsetHeight ?? 0) + (body?.scrollHeight ?? 0) + (footer?.offsetHeight ?? 0) + spacer
  return { width: Math.ceil(root.getBoundingClientRect().width), height: Math.ceil(height) }
}

export interface Size {
  width: number
  height: number
}

/**
 * Coalesces size reports: at most one per animation frame plus `delay` ms,
 * and only when the size changed by at least a pixel. Returns `schedule`
 * and `dispose`.
 */
export function coalescedReporter(
  measure: () => Size | null,
  report: (size: Size) => void,
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
    last = size
    report(size)
  }
  return {
    schedule: () => {
      if (pending || disposed) return
      pending = true
      requestAnimationFrame(() => {
        timer = setTimeout(flush, delay)
      })
    },
    dispose: () => {
      disposed = true
      if (timer) clearTimeout(timer)
    },
  }
}
