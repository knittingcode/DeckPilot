import { useCallback, useContext, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { useTranslation } from "@/i18n"
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
  const { t } = useTranslation()
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

  const profileName = profiles.profiles.find((p) => p.id === editingProfileId)?.name
  // The sentences name the profile being edited; without a name, "this profile".
  const sentence = {
    title: (layout: string) =>
      profileName === undefined
        ? t("layouts:open.titleThisProfile", { layout })
        : t("layouts:open.title", { layout, profile: profileName }),
    description:
      profileName === undefined
        ? t("layouts:open.descriptionThisProfile")
        : t("layouts:open.description", { profile: profileName }),
    confirm:
      profileName === undefined
        ? t("layouts:open.confirmThisProfile")
        : t("layouts:open.confirm", { profile: profileName }),
    failed: (layout: string) =>
      profileName === undefined
        ? t("layouts:open.failedThisProfile", { layout })
        : t("layouts:open.failed", { layout, profile: profileName }),
  }
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
                profileName: profileName ?? "",
                position: 0,
                isDefault: false,
              },
            ],
          },
          editingProfileId,
        )
      },
      (e: unknown) => {
        toast.error(sentence.failed(layout.name), {
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
      title={sentence.title(pending?.name ?? "")}
      description={sentence.description}
      confirmLabel={sentence.confirm}
      destructive={false}
      onConfirm={assignAndOpen}
    />
  )

  return { open, dialog }
}
