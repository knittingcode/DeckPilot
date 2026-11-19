import { useEffect, useRef } from "react"

import { type EditorCommand, editorCommand } from "@/lib/shortcuts"
import { useViewActive } from "@/shell/view-activity"

export type EditorShortcuts = Partial<Record<EditorCommand, () => void>>

/**
 * Window-wide editing shortcuts of an editor. Text fields, open dialogs and
 * selected text keep their own (native) undo, copy and paste; see
 * `editorCommand`. While the editor's view is hidden (the settings are
 * open over it) they do nothing.
 */
export function useEditorShortcuts(shortcuts: EditorShortcuts) {
  const active = useViewActive()
  const current = useRef(shortcuts)
  const activeRef = useRef(active)
  useEffect(() => {
    current.current = shortcuts
    activeRef.current = active
  })
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!activeRef.current) return
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
