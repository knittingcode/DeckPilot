import { useEffect, useRef } from "react"

import { type EditorCommand, editorCommand } from "@/lib/shortcuts"

export type EditorShortcuts = Partial<Record<EditorCommand, () => void>>

/**
 * Window-wide editing shortcuts of an editor. Text fields, open dialogs and
 * selected text keep their own (native) undo, copy and paste; see
 * `editorCommand`.
 */
export function useEditorShortcuts(shortcuts: EditorShortcuts) {
  const current = useRef(shortcuts)
  useEffect(() => {
    current.current = shortcuts
  })
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const command = editorCommand(event)
      const run = command && current.current[command]
      if (!run) return
      event.preventDefault()
      run()
    }
    window.addEventListener("keydown", onKey)
    return () => {
      window.removeEventListener("keydown", onKey)
    }
  }, [])
}
