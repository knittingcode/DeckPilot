import { useEffect, useState } from "react"

import { api } from "@/lib/api"
import { controlKey } from "@/lib/layout-model"
import type { ControlFace, DeviceLayout, KeyFace } from "@/lib/types"

/**
 * Faces and key images exactly as the agent would render them (with
 * appearance and icons), debounced while editing.
 */
export function usePreviewFaces(
  profileId: string,
  draft: DeviceLayout,
  pageId: string,
  project: string | null,
  /** Changes when the profile's icons change, so previews pick them up. */
  iconsVersion: string,
) {
  const [preview, setPreview] = useState({
    faces: new Map<string, KeyFace>(),
    images: new Map<string, string>(),
    /** Bindings kept although they cannot run from their control. */
    incompatible: new Map<string, NonNullable<ControlFace["incompatible"]>>(),
  })
  useEffect(() => {
    let cancelled = false
    const timer = setTimeout(() => {
      api.previewLayout(profileId, draft, pageId, project, true).then(
        (result) => {
          if (cancelled) return
          setPreview({
            faces: new Map(result.map((r) => [controlKey(r.control), r.face])),
            images: new Map(
              result.flatMap((r) => (r.image ? [[controlKey(r.control), r.image] as const] : [])),
            ),
            incompatible: new Map(
              result.flatMap((r) =>
                r.incompatible?.length ? [[controlKey(r.control), r.incompatible] as const] : [],
              ),
            ),
          })
        },
        () => undefined,
      )
    }, 150)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [profileId, draft, pageId, project, iconsVersion])
  return preview
}
