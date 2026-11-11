import { useDraggable } from "@dnd-kit/core"
import { ArrowLeft, Folder, ListOrdered } from "lucide-react"

import { ActionIcon } from "@/components/action-icon"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { cn } from "@/lib/utils"

import type { DragSource } from "./editing"
import type { TargetGroup, TargetOption } from "./targets"

function OptionIcon({ option }: { option: TargetOption }) {
  if (option.kind === "action") return <ActionIcon name={option.icon} className="size-4 shrink-0" />
  if (option.kind === "workflow") return <ListOrdered className="size-4 shrink-0" />
  if (option.target.type === "back") return <ArrowLeft className="size-4 shrink-0" />
  return <Folder className="size-4 shrink-0" />
}

function LibraryEntry({ option }: { option: TargetOption }) {
  const source: DragSource = { type: "target", target: option.target }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: `library:${option.value}`,
    data: source,
  })
  return (
    <li
      ref={setNodeRef}
      className={cn(
        "flex min-w-0 cursor-grab items-center gap-2 rounded-md border bg-card px-2 py-1.5 text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
        isDragging && "opacity-60",
      )}
      {...attributes}
      {...listeners}
      aria-label={`Drag ${option.label} onto a control`}
    >
      <OptionIcon option={option} />
      <span className="min-w-0 flex-1 truncate" title={option.label}>
        {option.label}
      </span>
    </li>
  )
}

/** Actions, workflows and navigation targets that can be dragged onto controls. */
export function LibraryPanel({ groups }: { groups: TargetGroup[] }) {
  const hasItems = groups.some((g) => g.label !== "Navigation")
  return (
    <Card className="h-fit xl:row-span-2 2xl:row-span-1">
      <CardHeader>
        <CardTitle className="text-sm">Library</CardTitle>
        <CardDescription>
          Drag onto a control, or select a control and choose in the inspector.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid max-h-[28rem] gap-3 overflow-y-auto">
        {!hasItems && (
          <p className="text-sm text-muted-foreground">
            No actions or workflows yet. Create them in the Action Library.
          </p>
        )}
        {groups.map((group) => (
          <section key={group.label} className="grid gap-1" aria-label={group.label}>
            <h3 className="text-xs font-medium text-muted-foreground">{group.label}</h3>
            <ul className="grid gap-1">
              {group.options.map((option) => (
                <LibraryEntry key={option.value} option={option} />
              ))}
            </ul>
          </section>
        ))}
      </CardContent>
    </Card>
  )
}
