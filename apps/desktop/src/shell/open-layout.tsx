import { useCallback, useContext, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { failureDetail } from "@/lib/failure"
import type { SharedLayoutSummary } from "@/lib/types"

import { EditorSelectionContext } from "./editor-selection-context"

/**
 * The profile to edit a shared layout in: the one being edited if it uses
 * the layout, else one that has it as default, else the first. `null`: no
 * profile uses it.
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

export type OpenableLayout = Pick<SharedLayoutSummary, "id" | "name" | "model" | "profiles">

/**
 * Opens a shared layout in the device editor. Used by the profile being
 * edited: selects it. Used by other profiles only: switches to one of
 * them. Used by none: asks to assign it to the profile being edited first
 * (render `dialog`). Refused, like any editor selection, while the editor
 * has unsaved changes.
 */
export function useOpenLayout() {
  const { editingProfileId, navigate, profiles, refresh } = useAppContext()
  const selection = useContext(EditorSelectionContext)
  const [pending, setPending] = useState<OpenableLayout | null>(null)

  const show = useCallback(
    (layout: OpenableLayout, profileId: string) => {
      if (selection) {
        // Another layout replaces the editor's: never over unsaved changes.
        if (layout.id !== selection.layoutId && !selection.guard()) return false
        if (!selection.selectProfile(profileId)) return false
        selection.showLayout(layout.id, layout.model)
      }
      navigate("designer")
      return true
    },
    [navigate, selection],
  )

  const open = useCallback(
    (layout: OpenableLayout) => {
      const profileId = profileToOpen(layout, editingProfileId)
      if (profileId !== null) return show(layout, profileId)
      if (editingProfileId === null) return false
      setPending(layout)
      return false
    },
    [editingProfileId, show],
  )

  const editingName =
    profiles.profiles.find((p) => p.id === editingProfileId)?.name ?? "this profile"
  const assignAndOpen = () => {
    const layout = pending
    setPending(null)
    if (!layout || editingProfileId === null) return
    api.assignLayout(editingProfileId, layout.id).then(
      () => {
        refresh()
        show(
          {
            ...layout,
            profiles: [
              {
                profileId: editingProfileId,
                profileName: editingName,
                position: 0,
                isDefault: false,
              },
            ],
          },
          editingProfileId,
        )
      },
      (e: unknown) => {
        toast.error(`“${layout.name}” was not added to ${editingName}`, {
          description: failureDetail(e),
        })
      },
    )
  }

  const dialog = (
    <ConfirmDialog
      open={pending !== null}
      onOpenChange={(next) => {
        if (!next) setPending(null)
      }}
      title={`Use “${pending?.name ?? ""}” in ${editingName}?`}
      description={`No profile uses this layout yet. It is added to the layouts of ${editingName} and opens in the Device Editor.`}
      confirmLabel={`Use in ${editingName}`}
      destructive={false}
      onConfirm={assignAndOpen}
    />
  )

  return { open, dialog }
}
