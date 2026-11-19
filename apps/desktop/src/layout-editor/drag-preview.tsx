import { FaceTile } from "@/components/key-face"

import type { DragLook } from "./drag-look"

/** The drag preview: a compact chip with the icon and name, or the dragged key. */
export function DragPreview({ look }: { look: DragLook }) {
  if (look.key) {
    return (
      <div
        data-slot="drag-preview"
        aria-hidden
        className="size-16 cursor-grabbing overflow-hidden rounded-[0.6rem] border border-device-key-border bg-device-key shadow-lg [--key:4rem]"
      >
        {look.key.image ? (
          <img src={look.key.image} alt="" className="size-full object-cover" />
        ) : (
          <FaceTile face={look.key.face} icon={look.key.icon} />
        )}
      </div>
    )
  }
  return (
    <div
      data-slot="drag-preview"
      aria-hidden
      className="inline-flex h-8 max-w-60 cursor-grabbing items-center gap-2 rounded-md border bg-popover px-2.5 text-sm font-medium text-popover-foreground shadow-md"
    >
      <span className="flex size-5 shrink-0 items-center justify-center rounded-sm bg-muted text-muted-foreground">
        {look.icon}
      </span>
      <span className="truncate">{look.label}</span>
    </div>
  )
}
