import { createContext, useContext } from "react"

import type { AsyncState } from "@/hooks/use-async"
import { i18n } from "@/i18n"
import type { EditorPlace } from "@/layout-editor/use-layout-editing"
import type { DeviceId, DeviceModel } from "@/lib/types"

/**
 * What the device editor shows, chosen in the main window's toolbar: the
 * device (a connected one, or a model from the catalog) and, in the
 * editor, the layout. The profile is the app's `editingProfileId`. Every
 * change goes through one guard, so unsaved layout changes are never lost
 * by switching the profile, the device or the layout.
 */
export interface EditorSelection {
  /** The catalog of device models. */
  models: AsyncState<DeviceModel[]>
  /** The chosen model; `null` until chosen (the editor picks one then). */
  modelId: string | null
  /** The connected device it was chosen through, if any. */
  device: DeviceId | null
  /** The chosen layout; `null`: the model's active layout. */
  layoutId: string | null
  /** Chooses a device or a model; refused while the layout has unsaved changes. */
  selectDevice: (model: string, device?: DeviceId | null) => boolean
  /** Chooses the profile to edit; refused likewise. */
  selectProfile: (id: string) => boolean
  /** Chooses a layout of the current model; refused likewise. */
  selectLayout: (id: string | null) => boolean
  /**
   * Shows `layout` (e.g. one just created, or opened from another page)
   * without asking: the model follows the layout.
   */
  showLayout: (layout: string, model?: string | null) => void
  /** The model the editor settled on while none was chosen. */
  adoptModel: (model: string) => void
  /** The editor reports whether its layout has unsaved changes. */
  setDirty: (dirty: boolean) => void
  /** Whether a change may happen now (warns if not). */
  guard: () => boolean
  /** The layout has unsaved changes. */
  isDirty: () => boolean
  /**
   * Where the editor was in a layout (`<profile>:<layout>`): its page and
   * selected keys, kept while other views (Settings, the Marketplace,
   * plugin views) are on screen, so it comes back as it was.
   */
  place: (layout: string) => EditorPlace | undefined
  rememberPlace: (layout: string, place: EditorPlace) => void
}

export const EditorSelectionContext = createContext<EditorSelection | null>(null)

/** The English text of `unsavedWarning()`; prefer the function, which follows the language. */
export const UNSAVED_WARNING = "Save or revert the current layout first"

/** Shown when a change of selection would discard unsaved edits, in the window's language. */
export function unsavedWarning(): string {
  return i18n.t("workspace:shell.unsavedWarning")
}

/** Whether the editor's layout has unsaved changes; owned by the shell. */
export interface DirtyFlag {
  current: boolean
}

/** The editor selection, if the editor is inside the shell (not in tests of it alone). */
export function useOptionalEditorSelection(): EditorSelection | null {
  return useContext(EditorSelectionContext)
}

export function useEditorSelection(): EditorSelection {
  const value = useContext(EditorSelectionContext)
  if (!value) throw new Error("useEditorSelection must be used inside EditorSelectionProvider")
  return value
}
