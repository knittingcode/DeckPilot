import { DestructiveDialog } from "@/components/dialogs"
import { useTranslation } from "@/i18n"

import { deletionNotes, type PageDeletion } from "./editing"

/**
 * Asks before a page is deleted, saying what goes with it. The deletion is
 * part of the draft: it can be undone until the layout is saved.
 */
export function DeletePageDialog({
  deletion,
  onCancel,
  onConfirm,
}: {
  /** The page to delete; `null`: the dialog is closed. */
  deletion: PageDeletion | null
  onCancel: () => void
  onConfirm: () => void
}) {
  const { t } = useTranslation()
  return (
    <DestructiveDialog
      open={deletion !== null}
      onOpenChange={(open) => {
        // Also called after a confirmation; clearing the request again is harmless.
        if (!open) onCancel()
      }}
      title={t("editor:deletePage.title", {
        page: deletion?.name || t("editor:pageNav.untitled"),
      })}
      description={
        <span className="grid gap-1">
          {deletion && deletionNotes(deletion).map((note) => <span key={note}>{note}</span>)}
          <span>{t("editor:deletePage.undoable")}</span>
        </span>
      }
      confirmLabel={t("editor:pageSettings.delete")}
      onConfirm={onConfirm}
    />
  )
}
