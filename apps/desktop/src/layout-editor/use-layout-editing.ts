import { useState } from "react"
import { toast } from "sonner"

import { i18n } from "@/i18n"
import { DEFAULT_AUTO_EXIT, RESERVED_BACK, withoutSystemBack } from "@/lib/folder-back"
import {
  addPage,
  controlLabel,
  deletePage,
  page as findPage,
  pageParents,
  renamePage,
  setAutoReturn,
} from "@/lib/layout-model"
import type { BindingTarget, ControlId, DeviceLayout, DeviceModel, Trigger } from "@/lib/types"

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

/** The folder `id` is in; `undefined` at the top level. */
function parentOf(layout: DeviceLayout, id: string): string | undefined {
  return pageParents(layout).get(id)
}

/** Where the editor is in a layout: kept by the shell while it is not on screen. */
export interface EditorPlace {
  pageId: string
  trail: string[]
  selection: Selection
}

/**
 * State of the layout editor: the draft with its undo history, the open
 * page and how it was reached, the selection and the clipboard.
 */
export function useLayoutEditing(
  model: DeviceModel,
  saved: DeviceLayout,
  /**
   * Whether `target` can run from `control` for `trigger`: assigning and
   * dropping prefer such triggers (docs/device-controls.md).
   */
  accepts?: (control: ControlId, target: BindingTarget, trigger: Trigger) => boolean,
  /**
   * Where the editor was in this layout when it was last on screen (the
   * shell keeps it while other views are): the open page, how it was
   * reached and the selection. A page that no longer exists opens the home
   * page.
   */
  initial?: EditorPlace,
) {
  const [state, setState] = useState(() => history(saved))
  const [openPage, setOpenPage] = useState(() =>
    initial && findPage(saved, initial.pageId) ? initial.pageId : saved.homePage,
  )
  // The stored layout the draft is compared with. Page commands (reorder,
  // move, copy) save on the backend and hand back the stored layout before
  // the `saved` prop catches up.
  const [baseline, setBaseline] = useState(saved)
  const [savedProp, setSavedProp] = useState(saved)
  if (saved !== savedProp) {
    setSavedProp(saved)
    setBaseline(saved)
  }
  const restored = initial !== undefined && findPage(saved, initial.pageId) !== undefined
  const [trail, setTrail] = useState<string[]>(() => (restored ? initial.trail : []))
  const [selection, setSelection] = useState<Selection>(() =>
    restored ? initial.selection : emptySelection,
  )
  const [clipboard, setClipboard] = useState<Clipboard | null>(null)
  // The page "Delete page" asked about, until it is confirmed or cancelled.
  const [deleting, setDeleting] = useState<string | null>(null)

  const draft = state.present
  // Undo or Revert can remove the open page: the nearest page it was
  // reached through that is still there (the folder it was in), else the
  // home page; as the device does (docs/editor-navigation-and-simulation.md).
  const currentPage =
    findPage(draft, openPage) ??
    [...trail].reverse().flatMap((id) => findPage(draft, id) ?? [])[0] ??
    findPage(draft, draft.homePage) ??
    draft.pages[0]
  const pageId = currentPage?.id ?? draft.homePage
  const selected = selectedInOrder(model, selection)
  // What edits of the selection change: never the folder's system Back,
  // which is the agent's (it stays selectable, for the inspector).
  const editable = withoutSystemBack(model, draft, pageId, selected)

  const change = (layout: DeviceLayout, group?: string) => {
    setState((s) => commit(s, layout, group ?? null))
  }

  const showPage = (id: string, nextTrail: string[]) => {
    setOpenPage(id)
    setTrail(nextTrail)
    setSelection(emptySelection)
  }

  const copy = () => {
    const copied = copyControls(model, draft, pageId, editable)
    if (!copied) return
    setClipboard(copied)
    toast.success(i18n.t("editor:editing.copied", { count: copied.entries.length }))
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
        commit(
          s,
          applyAppearance(
            s.present,
            pageId,
            withoutSystemBack(model, s.present, pageId, selected),
            look,
          ).layout,
          group ?? null,
        ),
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
      if (editable.length === 0) return
      copy()
      change(clearControls(draft, pageId, editable))
    },
    paste: () => {
      if (!clipboard || editable.length === 0) return
      const result = pasteControls(model, draft, pageId, clipboard, editable)
      change(result.layout)
      if (result.controls.length > 0) {
        setSelection({
          controls: result.controls,
          anchor: result.controls[0] ?? null,
          focus: result.controls.at(-1) ?? null,
        })
      }
      if (result.skipped > 0) {
        toast.warning(i18n.t("editor:editing.notPasted", { count: result.skipped }), {
          description: i18n.t("editor:editing.notPastedDetail"),
        })
      }
    },
    duplicate: () => {
      const result = duplicateControls(model, draft, pageId, editable)
      if (result.controls.length === 0) {
        toast.warning(i18n.t("editor:editing.noFreeControl"))
        return
      }
      change(result.layout)
      commands.selectAll(result.controls)
    },
    clear: () => {
      if (editable.length > 0) change(clearControls(draft, pageId, editable))
    },
    assign: (target: BindingTarget) => {
      change(
        assignTarget(
          model,
          draft,
          pageId,
          editable,
          target,
          accepts && ((control, trigger) => accepts(control, target, trigger)),
        ),
      )
    },
    /**
     * Creates a folder in the open page and makes `control` open it, as one
     * undo step. A control that already does something is left alone.
     */
    newFolder: (control: ControlId) => {
      const next = createFolder(model, draft, pageId, control, crypto.randomUUID())
      if (!next) {
        toast.error(i18n.t("editor:editing.folderTaken", { control: controlLabel(control) }), {
          description: i18n.t("editor:editing.folderTakenDetail"),
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
      // An action type and a Multi Action are placed by the editor, which
      // creates the action or the workflow.
      if (source.type === "actionType" || source.type === "newMultiAction") return
      const next = dropOnControl(
        model,
        draft,
        pageId,
        source,
        over,
        accepts && source.type === "target"
          ? (control, trigger) => accepts(control, source.target, trigger)
          : undefined,
      )
      if (!next) {
        toast.error(i18n.t("editor:editing.dropRefused", { control: controlLabel(over) }))
        return
      }
      change(next)
      setSelection(selectOnly(over))
    },
    openPage: (id: string) => {
      showPage(id, [])
    },
    /** Shows `id` as reached through `through` (a breadcrumb). */
    openAt: (id: string, through: string[]) => {
      if (findPage(draft, id))
        showPage(
          id,
          through.filter((p) => p !== id && findPage(draft, p)),
        )
    },
    /** Shows `id` as opened from the page shown (a folder key). */
    enter: (id: string) => {
      if (findPage(draft, id) && id !== pageId) showPage(id, [...trail, pageId])
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
    /**
     * Adds a page and shows it: in `folder` (a page id), else in the folder
     * the open page is in (the top level for a top-level page).
     */
    addPage: (name: string, folder?: string | null) => {
      const id = crypto.randomUUID()
      const parent =
        folder === undefined
          ? pageId === draft.homePage
            ? undefined
            : parentOf(draft, pageId)
          : folder
      const added = addPage(draft, id, name)
      change(
        parent && findPage(draft, parent)
          ? {
              ...added,
              // A page inside a folder is a folder page: protected Back,
              // Auto Exit off (as `createFolder`).
              pages: added.pages.map((p) =>
                p.id === id
                  ? {
                      ...p,
                      parent,
                      back: { ...RESERVED_BACK },
                      autoExit: { ...DEFAULT_AUTO_EXIT },
                    }
                  : p,
              ),
            }
          : added,
      )
      showPage(id, [])
    },
    /** The one rename of a page (the breadcrumb and the page navigator use it). */
    renamePage: (name: string, id: string = pageId) => {
      if (!findPage(draft, id)) return
      change(renamePage(draft, id, name), `page-name:${id}`)
    },
    setAutoReturn: (on: boolean, id: string = pageId) => {
      change(setAutoReturn(draft, id, on))
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
    deletePage: (id: string = pageId) => {
      if (id !== draft.homePage && findPage(draft, id)) setDeleting(id)
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
    // Up to the page shown, when the page opened is gone (see `currentPage`).
    trail: (findPage(draft, openPage)
      ? trail
      : trail.slice(0, Math.max(0, trail.indexOf(pageId)))
    ).filter((id) => findPage(draft, id)),
    selection,
    selected,
    /** The selection without the folder's system Back: what edits change. */
    editable,
    clipboard,
    /** The page whose deletion waits for confirmation, if any. */
    deleting: deleting && findPage(draft, deleting) ? deleting : null,
    canUndo: canUndo(state),
    canRedo: canRedo(state),
    commands,
  }
}
