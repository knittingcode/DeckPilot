import { Trash2 } from "lucide-react"
import type { ReactNode } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Switch } from "@/components/ui/switch"
import { ancestors, page as findPage } from "@/lib/layout-model"
import type { DeviceLayout } from "@/lib/types"

interface PageSettingsProps {
  draft: DeviceLayout
  pageId: string
  onRename: (name: string) => void
  onDelete: () => void
  /** Makes the open folder close after one of its keys ran something. */
  onAutoReturn: (on: boolean) => void
}

/** The open page's name, "Delete page" and, for a folder, closing after use. */
export function PageSettings({
  draft,
  pageId,
  onRename,
  onDelete,
  onAutoReturn,
}: PageSettingsProps) {
  const current = findPage(draft, pageId)
  const isHome = pageId === draft.homePage
  // Only a page inside a folder has somewhere to return to.
  const inFolder = ancestors(draft, pageId).length > 0
  if (!current) return null
  return (
    <section className="grid gap-3" aria-label="This page">
      <h3 className="text-xs font-medium text-muted-foreground uppercase">This page</h3>
      {isHome ? (
        <p className="text-sm">
          <span className="font-medium">{current.name || "Untitled"}</span>{" "}
          <span className="text-muted-foreground">
            is the home page, shown first; it is not deleted.
          </span>
        </p>
      ) : (
        <>
          <div className="grid gap-1.5">
            <Label htmlFor="page-name">Page name</Label>
            <div className="flex items-center gap-2">
              <Input
                id="page-name"
                className="h-8 flex-1"
                value={current.name}
                onChange={(e) => {
                  onRename(e.target.value)
                }}
              />
              <Button variant="ghost" size="sm" onClick={onDelete}>
                <Trash2 /> Delete page
              </Button>
            </div>
          </div>
          {inFolder && (
            <label
              className="flex items-center gap-2 text-sm"
              title="After a key of this folder ran an action or started a workflow, the device returns to the page the folder is on. Keys that open pages do not close it."
            >
              <Switch
                aria-label="Close this folder after a key runs"
                checked={current.autoReturn === true}
                onCheckedChange={onAutoReturn}
              />
              Close after a key runs
            </label>
          )}
        </>
      )}
    </section>
  )
}

/** The pages panel: the open page's settings above the page tree. */
export function PagesSheet({
  open,
  onOpenChange,
  children,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  children: ReactNode
}) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="gap-0 sm:max-w-md">
        <SheetHeader className="border-b">
          <SheetTitle>Pages</SheetTitle>
          <SheetDescription>
            Rename or delete the open page; reorder, move or copy pages in the tree.
          </SheetDescription>
        </SheetHeader>
        <div className="grid min-h-0 flex-1 content-start gap-6 overflow-y-auto p-4">
          {children}
        </div>
      </SheetContent>
    </Sheet>
  )
}
