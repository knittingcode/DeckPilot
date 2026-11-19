import { Library } from "lucide-react"
import { type ReactNode } from "react"

/**
 * The window width below which the library sits under the editor instead
 * of beside it (CSS pixels): the main window's minimum width
 * (`tauri.conf.json`), reached only where a window manager ignores it.
 */
export const LIBRARY_BESIDE_FROM = 860

/**
 * The Action Library at the right of the device editor: always shown, one
 * fixed width (the `--library-width` token), never collapsed or resized,
 * so it keeps its place while pages and controls change. Below
 * `LIBRARY_BESIDE_FROM` the editor stacks it under the device instead
 * (`device-editor` in editor.tsx). Dragging from it onto the device works
 * in both, as it stays inside the editor's drag and drop context.
 */
export function LibraryDock({ children }: { children: ReactNode }) {
  return (
    <aside
      aria-label="Action Library"
      data-slot="library-dock"
      className="flex min-h-0 shrink-0 flex-col border-t bg-sidebar max-[859px]:h-(--library-stacked-height) max-[859px]:w-full min-[860px]:w-(--library-width) min-[860px]:border-t-0 min-[860px]:border-l"
    >
      <div className="flex h-11 shrink-0 items-center gap-2 border-b px-3">
        <Library className="size-4 text-muted-foreground" aria-hidden />
        <h2 className="flex-1 text-sm font-semibold">Library</h2>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-3">{children}</div>
    </aside>
  )
}
