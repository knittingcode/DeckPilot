import { useEffect, useState } from "react"

import { usePrefersReducedMotion } from "@/hooks/use-reduced-motion"
import { api } from "@/lib/api"
import { parseControlKey } from "@/layout-editor/editing"
import type { DeviceLayout, LivePreview, MarqueePreview, TextLayoutReport } from "@/lib/types"

/**
 * The frames of the page's scrolling titles, as the device shows them
 * (`previewMarquee`, docs/key-text.md), for keys whose report says they
 * scroll. None while the system asks for reduced motion: the keys keep
 * their still images (the title cut with `…`).
 */
export function useMarqueeFrames(
  profileId: string,
  draft: DeviceLayout,
  pageId: string,
  reports: Map<string, TextLayoutReport>,
  live: LivePreview | null,
  keyPixels: number | null,
): Map<string, MarqueePreview> {
  const reduced = usePrefersReducedMotion()
  // The frames with what they were made for: those of an earlier draft
  // are not played while the new ones are made.
  const [made, setMade] = useState<{
    draft: DeviceLayout
    pageId: string
    keys: string
    frames: Map<string, MarqueePreview>
  } | null>(null)
  const scrolling = reduced
    ? []
    : [...reports].filter(([, report]) => report.needsScroll).map(([key]) => key)
  const keys = scrolling.join("|")
  useEffect(() => {
    let cancelled = false
    if (!keys) return
    const timer = setTimeout(() => {
      const wanted = keys.split("|")
      void Promise.all(
        wanted.map(async (key) => {
          const control = parseControlKey(key)
          if (!control) return null
          try {
            const preview = await api.previewMarquee(
              profileId,
              draft,
              pageId,
              control,
              live,
              keyPixels,
            )
            return preview && preview.frames.length > 0 ? ([key, preview] as const) : null
          } catch {
            return null
          }
        }),
      ).then((found) => {
        if (!cancelled)
          setMade({
            draft,
            pageId,
            keys,
            frames: new Map(found.filter((entry) => entry !== null)),
          })
      })
    }, 300)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [profileId, draft, pageId, keys, live, keyPixels])
  return made && keys && made.draft === draft && made.pageId === pageId && made.keys === keys
    ? made.frames
    : NONE
}

const NONE = new Map<string, MarqueePreview>()
