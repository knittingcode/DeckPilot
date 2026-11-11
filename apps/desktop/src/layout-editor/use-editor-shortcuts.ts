import { useEffect, useRef } from "react"

import { IS_MAC } from "@/hooks/use-navigation"

export interface EditorShortcuts {
  undo: () => void
  redo: () => void
  copy: () => void
  cut: () => void
  paste: () => void
  duplicate: () => void
}

/** "⌘C" on macOS, "Ctrl+C" elsewhere. */
export function shortcutLabel(key: string, shift = false): string {
  return IS_MAC ? `${shift ? "⇧" : ""}⌘${key}` : `Ctrl+${shift ? "Shift+" : ""}${key}`
}

function isEditable(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName))
  )
}

/**
 * Window-wide editing shortcuts of the layout editor. Text fields keep
 * their own undo, copy and paste.
 */
export function useEditorShortcuts(shortcuts: EditorShortcuts) {
  const current = useRef(shortcuts)
  useEffect(() => {
    current.current = shortcuts
  })
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const mod = IS_MAC ? event.metaKey : event.ctrlKey
      if (!mod || event.altKey || event.defaultPrevented || isEditable(event.target)) return
      // A dialog or menu above the editor owns the keyboard.
      if (document.querySelector("[role=dialog], [role=alertdialog], [role=menu]")) return
      const key = event.key.toLowerCase()
      // Copying selected page text stays with the browser.
      if ((key === "c" || key === "x") && window.getSelection()?.toString()) return
      const run = current.current
      if (key === "z" && !event.shiftKey) run.undo()
      else if ((key === "z" && event.shiftKey) || (key === "y" && !event.shiftKey)) run.redo()
      else if (key === "c" && !event.shiftKey) run.copy()
      else if (key === "x" && !event.shiftKey) run.cut()
      else if (key === "v" && !event.shiftKey) run.paste()
      else if (key === "d" && !event.shiftKey) run.duplicate()
      else return
      event.preventDefault()
    }
    window.addEventListener("keydown", onKey)
    return () => {
      window.removeEventListener("keydown", onKey)
    }
  }, [])
}
