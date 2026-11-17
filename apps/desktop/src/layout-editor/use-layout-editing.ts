import { useState } from "react"
import { toast } from "sonner"

import {
  addPage,
  controlLabel,
  deletePage,
  page as findPage,
  renamePage,
  setAutoReturn,
} from "@/lib/layout-model"
import type { BindingTarget, ControlId, DeviceLayout, DeviceModel } from "@/lib/types"

import { type KeyLookChange, applyAppearance } from "./appearance"
import { canRedo, canUndo, commit, history, redo, undo } from "@/lib/history"

import {
  assignTarget,
  clearControls,
  copyControls,
  createFolder,
  dropOnControl,
  duplicateControls,
  emptySelection,
  folderOf,
  isSelected,
  parentPage,
  pasteControls,
  select,
  selectOnly,
  selectedInOrder,
  type Clipboard,
  type DragSource,
  type SelectMode,
  type Selection,
} from "./editing"

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`

/**
 * State of the layout editor: the draft with its undo history, the open
 * page and how it was reached, the selection and the clipboard.
 */
export function useLayoutEditing(model: DeviceModel, saved: DeviceLayout) {
  const [state, setState] = useState(() => history(saved))
  const [openPage, setOpenPage] = useState(saved.homePage)
  // The stored layout the draft is compared with. Page commands (reorder,
  // move, copy) save on the backend and hand back the stored layout before
  // the `saved` prop catches up.
  const [baseline, setBaseline] = useState(saved)
  const [savedProp, setSavedProp] = useState(saved)
  if (saved !== savedProp) {
    setSavedProp(saved)
    setBaseline(saved)
  }
  const [trail, setTrail] = useState<string[]>([])
  const [selection, setSelection] = useState<Selection>(emptySelection)
  const [clipboard, setClipboard] = useState<Clipboard | null>(null)
  // The page "Delete page" asked about, until it is confirmed or cancelled.
  const [deleting, setDeleting] = useState<string | null>(null)

  const draft = state.present
  // Undo can remove the open page; fall back to the home page then.
  const currentPage = findPage(draft, openPage) ?? findPage(draft, draft.homePage) ?? draft.pages[0]
  const pageId = currentPage?.id ?? draft.homePage
  const selected = selectedInOrder(model, selection)

  const change = (layout: DeviceLayout, group?: string) => {
    setState((s) => commit(s, layout, group ?? null))
  }

  const showPage = (id: string, nextTrail: string[]) => {
    setOpenPage(id)
    setTrail(nextTrail)
    setSelection(emptySelection)
  }

  const copy = () => {
    const copied = copyControls(model, draft, pageId, selected)
    if (!copied) return
    setClipboard(copied)
    toast.success(`Copied ${plural(copied.entries.length, "control")}`)
  }

  const commands = {
    change,
    /**
     * Changes the look of the selected keys. Applied to the latest draft, so
     * a change that arrives later (after an icon import) does not undo edits
     * made meanwhile.
     */
    restyle: (look: KeyLookChange, group?: string) => {
      setState((s) =>
        commit(s, applyAppearance(s.present, pageId, selected, look).layout, group ?? null),
      )
    },
    select: (control: ControlId, mode: SelectMode) => {
      setSelection((s) => select(model, s, control, mode))
    },
    /** Right-click: keeps a selection containing the control. */
    selectForMenu: (control: ControlId) => {
      setSelection((s) => (isSelected(s, control) ? { ...s, focus: control } : selectOnly(control)))
    },
    selectAll: (controls: ControlId[]) => {
      setSelection({
        controls,
        anchor: controls[0] ?? null,
        focus: controls.at(-1) ?? null,
      })
    },
    clearSelection: () => {
      setSelection(emptySelection)
    },
    undo: () => {
      setState(undo)
    },
    redo: () => {
      setState(redo)
    },
    revert: () => {
      change(baseline)
    },
    copy,
    cut: () => {
      if (selected.length === 0) return
      copy()
      change(clearControls(draft, pageId, selected))
    },
    paste: () => {
      if (!clipboard || selected.length === 0) return
      const result = pasteControls(model, draft, pageId, clipboard, selected)
      change(result.layout)
      if (result.controls.length > 0) {
        setSelection({
          controls: result.controls,
          anchor: result.controls[0] ?? null,
          focus: result.controls.at(-1) ?? null,
        })
      }
      if (result.skipped > 0) {
        toast.warning(`${plural(result.skipped, "binding")} could not be pasted here`, {
          description: "The control does not support that event, or the target page is missing.",
        })
      }
    },
    duplicate: () => {
      const result = duplicateControls(model, draft, pageId, selected)
      if (result.controls.length === 0) {
        toast.warning("No free control to duplicate onto")
        return
      }
      change(result.layout)
      commands.selectAll(result.controls)
    },
    clear: () => {
      if (selected.length > 0) change(clearControls(draft, pageId, selected))
    },
    assign: (target: BindingTarget) => {
      change(assignTarget(model, draft, pageId, selected, target))
    },
    /**
     * Creates a folder in the open page and makes `control` open it, as one
     * undo step. A control that already does something is left alone.
     */
    newFolder: (control: ControlId) => {
      const next = createFolder(model, draft, pageId, control, crypto.randomUUID())
      if (!next) {
        toast.error(`${controlLabel(control)} already does something`, {
          description: "Create the folder on an empty control, or clear this one first.",
        })
        return
      }
      change(next)
      setSelection(selectOnly(control))
    },
    drop: (source: DragSource, over: ControlId) => {
      if (source.type === "newFolder") {
        commands.newFolder(over)
        return
      }
      // An action type is placed by the editor, which creates its action.
      if (source.type === "actionType") return
      const next = dropOnControl(model, draft, pageId, source, over)
      if (!next) {
        toast.error(`${controlLabel(over)} cannot take these bindings`)
        return
      }
      change(next)
      setSelection(selectOnly(over))
    },
    openPage: (id: string) => {
      showPage(id, [])
    },
    openFolder: (control: ControlId) => {
      const folder = folderOf(draft, pageId, control)
      if (folder) showPage(folder, [...trail, pageId])
    },
    back: () => {
      const previous = trail.at(-1)
      if (previous && findPage(draft, previous)) showPage(previous, trail.slice(0, -1))
      else showPage(parentPage(draft, pageId), [])
    },
    addPage: (name: string) => {
      const id = crypto.randomUUID()
      change(addPage(draft, id, name))
      showPage(id, [])
    },
    renamePage: (name: string) => {
      change(renamePage(draft, pageId, name), `page-name:${pageId}`)
    },
    setAutoReturn: (on: boolean) => {
      change(setAutoReturn(draft, pageId, on))
    },
    /**
     * Replaces the draft and its history with a layout stored by a page
     * command, and shows `page` (else the home page).
     */
    reset: (layout: DeviceLayout, page?: string) => {
      setState(history(layout))
      setBaseline(layout)
      showPage(page && findPage(layout, page) ? page : layout.homePage, [])
    },
    /** Asks before deleting the open page; see `confirmDeletePage`. */
    deletePage: () => {
      if (pageId !== draft.homePage) setDeleting(pageId)
    },
    cancelDeletePage: () => {
      setDeleting(null)
    },
    /**
     * Deletes the page that was asked about and shows the page it was in
     * (else the home page).
     */
    confirmDeletePage: () => {
      const id = deleting
      setDeleting(null)
      if (!id || !findPage(draft, id)) return
      const folder = parentPage(draft, id)
      change(deletePage(draft, id))
      showPage(folder !== id && findPage(draft, folder) ? folder : draft.homePage, [])
    },
  }

  return {
    draft,
    baseline,
    pageId,
    trail: trail.filter((id) => findPage(draft, id)),
    selection,
    selected,
    clipboard,
    /** The page whose deletion waits for confirmation, if any. */
    deleting: deleting && findPage(draft, deleting) ? deleting : null,
    canUndo: canUndo(state),
    canRedo: canRedo(state),
    commands,
  }
}
