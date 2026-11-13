// Editing shortcuts (undo, copy, paste, …) of the layout editor and the
// Workflow Builder, and who owns them.
//
// On macOS the native Edit menu has the same key equivalents. WebKit gives
// the page the keydown first: when an editor handles a shortcut it calls
// `preventDefault()` and the menu item does not run; when it leaves the
// event alone the menu item runs on the focused element (text undo, copy of
// selected text). So an editor must only take a shortcut that is really its
// own and must leave text fields, open dialogs and selected text alone.

import { IS_MAC } from "@/hooks/use-navigation"

export type EditorCommand = "undo" | "redo" | "copy" | "cut" | "paste" | "duplicate" | "save"

/** Input types without text editing: shortcuts on them go to the editor. */
const NON_TEXT_INPUTS = new Set([
  "button",
  "checkbox",
  "color",
  "file",
  "image",
  "radio",
  "range",
  "reset",
  "submit",
])

/** Whether `target` edits text itself (and so keeps the native shortcuts). */
export function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  if (target instanceof HTMLInputElement) return !NON_TEXT_INPUTS.has(target.type)
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true
  return (
    target.isContentEditable ||
    target.closest("[contenteditable]:not([contenteditable=false])") !== null
  )
}

/** A dialog or menu is open above the editor and owns the keyboard. */
function overlayOpen(target: EventTarget | null): boolean {
  const overlay = "[role=dialog], [role=alertdialog], [role=menu]"
  return (
    (target instanceof Element && target.closest(overlay) !== null) ||
    document.querySelector(overlay) !== null
  )
}

function hasTextSelection(): boolean {
  const selection = window.getSelection()
  return selection !== null && !selection.isCollapsed && selection.toString() !== ""
}

/**
 * The editor command of a keydown, or `null` when the event belongs to
 * someone else: a text field, an open dialog or menu, a text selection
 * (copy and cut) or a handler that already took it.
 */
export function editorCommand(event: KeyboardEvent): EditorCommand | null {
  const mod = IS_MAC ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey
  if (!mod || event.altKey || event.defaultPrevented || event.isComposing) return null
  if (overlayOpen(event.target)) return null
  const key = event.key.toLowerCase()
  // Saving works from text fields too: it does not touch their text.
  if (key === "s" && !event.shiftKey) return "save"
  if (isTextEntry(event.target)) return null
  if (key === "z") return event.shiftKey ? "redo" : "undo"
  if (event.shiftKey) return null
  if (key === "y") return "redo"
  if (key === "c" || key === "x") {
    // Copying selected page text stays with the browser.
    if (hasTextSelection()) return null
    return key === "c" ? "copy" : "cut"
  }
  if (key === "v") return "paste"
  if (key === "d") return "duplicate"
  return null
}

/** "⌘C" on macOS, "Ctrl+C" elsewhere. */
export function shortcutLabel(key: string, shift = false): string {
  return IS_MAC ? `${shift ? "⇧" : ""}⌘${key}` : `Ctrl+${shift ? "Shift+" : ""}${key}`
}
