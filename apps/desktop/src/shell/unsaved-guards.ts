// Unsaved layout edits are never discarded without asking
// (docs/editor-navigation-and-simulation.md, section 3): leaving the
// device editor for another workspace page, closing the main window
// (Windows, Linux: it ends the editor; macOS hides it and keeps it) and
// quitting the app (⌘Q). The device's live preview ends with the editor,
// and the device shows the saved configuration again.

import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { type RefObject, useCallback, useContext, useEffect } from "react"
import { toast } from "sonner"

import { DialogsContext } from "@/components/dialogs/use-dialogs"
import { IS_MAC } from "@/hooks/use-navigation"
import { i18n } from "@/i18n"
import { locationOf } from "@/lib/navigation"

import { unsavedWarning } from "./editor-selection-context"

/** The backend's event asking to confirm quitting (`commands/unsaved.rs`). */
export const QUIT_REQUESTED_EVENT = "quit-requested"

/** The page the device editor is. */
const DESIGNER = "designer"

/** Whether going to `page` from `from` leaves the device editor. */
export function leavesEditor(from: { mode: string; page: string }, page: string): boolean {
  const to = locationOf(page)
  return (
    from.mode === "workspace" &&
    from.page === DESIGNER &&
    to.mode === "workspace" &&
    to.page !== DESIGNER
  )
}

/** Tells the app's backend whether there are unsaved edits (quitting asks then). */
export function reportUnsaved(unsaved: boolean) {
  void invoke("set_unsaved_edits", { unsaved }).catch(() => undefined)
}

/**
 * `navigate` that asks before leaving the device editor with unsaved
 * edits, and the close and quit guards of the main window.
 */
export function useUnsavedGuards(
  dirtyRef: RefObject<boolean>,
  location: { mode: string; page: string },
  navigate: (page: string) => void,
  setDirty: (dirty: boolean) => void,
) {
  const dialogs = useContext(DialogsContext)
  const confirm = useCallback(async () => {
    if (!dirtyRef.current) return true
    if (!dialogs) {
      toast.warning(unsavedWarning())
      return false
    }
    const discard = await dialogs.destructive({
      title: i18n.t("editor:editing.discard.title"),
      description: i18n.t("editor:editing.discard.description"),
      confirmLabel: i18n.t("editor:editing.discard.confirm"),
    })
    if (discard) setDirty(false)
    return discard
  }, [dirtyRef, dialogs, setDirty])

  const guarded = useCallback(
    (page: string) => {
      if (!dirtyRef.current || !leavesEditor(location, page)) {
        navigate(page)
        return
      }
      void confirm().then((ok) => {
        if (ok) navigate(page)
      })
    },
    [confirm, dirtyRef, location, navigate],
  )

  // Closing the main window ends the editor, except on macOS (it hides).
  useEffect(() => {
    if (IS_MAC) return
    let stop: (() => void) | undefined
    let stopped = false
    let window: ReturnType<typeof getCurrentWindow>
    try {
      window = getCurrentWindow()
    } catch {
      return
    }
    window
      .onCloseRequested(async (event) => {
        if (!dirtyRef.current) return
        event.preventDefault()
        if (await confirm()) void invoke("close_main_window").catch(() => undefined)
      })
      .then(
        (unlisten) => {
          if (stopped) unlisten()
          else stop = unlisten
        },
        () => undefined,
      )
    return () => {
      stopped = true
      stop?.()
    }
  }, [confirm, dirtyRef])

  // ⌘Q (and quitting from the app menu) over unsaved edits.
  useEffect(() => {
    let stop: (() => void) | undefined
    let stopped = false
    listen(QUIT_REQUESTED_EVENT, () => {
      void confirm().then((ok) => {
        if (ok) void invoke("quit_app").catch(() => undefined)
      })
    }).then(
      (unlisten) => {
        if (stopped) unlisten()
        else stop = unlisten
      },
      () => undefined,
    )
    return () => {
      stopped = true
      stop?.()
    }
  }, [confirm])

  return guarded
}
