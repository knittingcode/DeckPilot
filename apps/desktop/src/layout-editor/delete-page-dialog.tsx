import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"

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
  return (
    <AlertDialog
      open={deletion !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete “{deletion?.name || "Untitled"}”?</AlertDialogTitle>
          <AlertDialogDescription asChild>
            <div className="grid gap-1">
              {deletion && deletionNotes(deletion).map((note) => <p key={note}>{note}</p>)}
              <p>You can undo this until the layout is saved.</p>
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-white hover:bg-destructive/90"
            onClick={onConfirm}
          >
            Delete page
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
