import { useContext } from "react"

import { DialogsContext } from "@/components/dialogs/use-dialogs"
import { i18n } from "@/i18n"
import { useEditorSelection } from "@/shell/editor-selection-context"

/**
 * The layout the editor shows of `layouts` (the profile's for one model):
 * the chosen one, else the profile's default for the model, else the first.
 */
export function resolveLayout<T extends { id: string }>(
  layouts: T[],
  chosen: string | null,
  defaultLayout: string | null,
): T | undefined {
  return (
    layouts.find((l) => l.id === chosen) ??
    layouts.find((l) => l.id === defaultLayout) ??
    layouts[0]
  )
}

/**
 * The layout the editor opens when none is chosen: the one the device
 * shows when it runs the profile being edited (the editor follows the
 * device, docs/editor-navigation-and-simulation.md), else the profile's
 * default for the model, else its first.
 */
export function defaultLayoutOf(
  layouts: { id: string }[],
  profileDefault: string | null | undefined,
  shown: string | null,
  deviceRunsProfile: boolean,
): string | null {
  const followed = deviceRunsProfile && layouts.some((l) => l.id === shown) ? shown : null
  return followed ?? profileDefault ?? layouts[0]?.id ?? null
}

/**
 * Asks before unsaved layout changes are discarded (switching the layout,
 * the profile or the device, renaming): `true` when there are none or the
 * user discards them. Without the dialog service (a page rendered alone in
 * tests), the shell's guard refuses with its warning instead.
 */
export function useConfirmDiscard(): () => Promise<boolean> {
  const dialogs = useContext(DialogsContext)
  const selection = useEditorSelection()
  return async () => {
    if (!selection.isDirty()) return true
    if (!dialogs) return selection.guard()
    const discard = await dialogs.destructive({
      title: i18n.t("editor:editing.discard.title"),
      description: i18n.t("editor:editing.discard.description"),
      confirmLabel: i18n.t("editor:editing.discard.confirm"),
    })
    if (discard) selection.setDirty(false)
    return discard
  }
}
