import { useState } from "react"
import { toast } from "sonner"

import { addPage, controlLabel, deletePage, page as findPage, renamePage } from "@/lib/layout-model"
import type { BindingTarget, ControlId, DeviceLayout, DeviceModel } from "@/lib/types"

import { type KeyLookChange, applyAppearance } from "./appearance"
import {
  assignTarget,
  canRedo,
  canUndo,
  clearControls,
  commit,
  copyControls,
  dropOnControl,
  duplicateControls,
  emptySelection,
  folderOf,
  history,
  isSelected,
  parentPage,
  pasteControls,
  redo,
  select,
  selectOnly,
  selectedInOrder,
  undo,
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
  const [trail, setTrail] = useState<string[]>([])
  const [selection, setSelection] = useState<Selection>(emptySelection)
  const [clipboard, setClipboard] = useState<Clipboard | null>(null)

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
      change(saved)
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
    drop: (source: DragSource, over: ControlId) => {
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
    deletePage: () => {
      change(deletePage(draft, pageId))
      showPage(draft.homePage, [])
    },
  }

  return {
    draft,
    pageId,
    trail: trail.filter((id) => findPage(draft, id)),
    selection,
    selected,
    clipboard,
    canUndo: canUndo(state),
    canRedo: canRedo(state),
    commands,
  }
}
