import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"

/** The profile being edited, reloaded whenever data changes. */
export function useEditingProfile() {
  const { editingProfileId, revision } = useAppContext()
  const { state, reload } = useAsync(
    () =>
      editingProfileId
        ? api.getProfile(editingProfileId)
        : Promise.reject(new Error("No profile selected")),
    `${editingProfileId ?? "none"}:${revision}`,
  )
  return { state, reload, profileId: editingProfileId }
}
