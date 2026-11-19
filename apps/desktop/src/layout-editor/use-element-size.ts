import { useEffect, useState, type RefObject } from "react"

/**
 * The content box of an element, followed as it resizes; `null` until it
 * was measured (and where ResizeObserver is missing, e.g. in tests).
 */
export function useElementSize(
  ref: RefObject<HTMLElement | null>,
): { width: number; height: number } | null {
  const [size, setSize] = useState<{ width: number; height: number } | null>(null)
  useEffect(() => {
    const element = ref.current
    if (!element || typeof ResizeObserver === "undefined") return
    const observer = new ResizeObserver((entries) => {
      const box = entries[0]?.contentRect
      if (!box) return
      setSize((current) =>
        current && current.width === box.width && current.height === box.height
          ? current
          : { width: box.width, height: box.height },
      )
    })
    observer.observe(element)
    return () => {
      observer.disconnect()
    }
  }, [ref])
  return size
}
