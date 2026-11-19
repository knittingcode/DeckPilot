import { useCallback, useContext } from "react"

import { useAppContext } from "@/lib/app-context"
import type { SharedLayoutSummary } from "@/lib/types"

import { EditorSelectionContext } from "./editor-selection-context"

/**
 * The profile to edit a shared layout in: the one being edited if it uses
 * the layout, else one that has it as default, else the first. `null`: no
 * profile uses it (it cannot be opened in the device editor yet).
 */
export function profileToOpen(
  layout: Pick<SharedLayoutSummary, "profiles">,
  editingProfileId: string | null,
): string | null {
  const { profiles } = layout
  return (
    profiles.find((p) => p.profileId === editingProfileId)?.profileId ??
    profiles.find((p) => p.isDefault)?.profileId ??
    profiles[0]?.profileId ??
    null
  )
}

/**
 * Opens a shared layout in the device editor: switches to a profile that
 * uses it, selects the layout (its model follows) and shows the editor.
 * Refused, like any editor selection, while the editor has unsaved changes.
 *
 * phase 2: the device editor still reads layouts through the profile; once
 * it edits shared layouts directly, opening needs no profile switch.
 */
export function useOpenLayout() {
  const { editingProfileId, navigate } = useAppContext()
  const selection = useContext(EditorSelectionContext)
  return useCallback(
    (layout: Pick<SharedLayoutSummary, "id" | "model" | "profiles">) => {
      const profileId = profileToOpen(layout, editingProfileId)
      if (profileId === null) return false
      if (selection) {
        if (!selection.selectProfile(profileId)) return false
        selection.showLayout(layout.id, layout.model)
      }
      navigate("designer")
      return true
    },
    [editingProfileId, navigate, selection],
  )
}
