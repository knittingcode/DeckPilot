import { ArrowLeft, Folder, ListOrdered } from "lucide-react"

import { ActionIcon } from "@/components/action-icon"
import type { KeyFace } from "@/lib/types"
import { cn } from "@/lib/utils"

const STYLE: Record<KeyFace["style"], string> = {
  empty: "bg-neutral-950 text-neutral-500",
  action: "bg-neutral-800 text-neutral-50",
  workflow: "bg-slate-700 text-neutral-50",
  folder: "bg-indigo-900 text-neutral-50",
  back: "bg-neutral-700 text-neutral-50",
  notice: "bg-amber-950 text-amber-100",
}

/** A key or touch region as it would appear on the device. */
export function FaceTile({
  face,
  icon,
  compact = false,
  className,
}: {
  face: KeyFace | undefined
  icon?: string | null
  /** Text only, for short regions such as touch-strip zones. */
  compact?: boolean
  className?: string
}) {
  const style = face?.style ?? "empty"
  const Glyph =
    style === "folder"
      ? Folder
      : style === "back"
        ? ArrowLeft
        : style === "workflow"
          ? ListOrdered
          : null
  return (
    <div
      className={cn(
        "flex size-full flex-col items-center justify-center gap-0.5 overflow-hidden rounded-md p-1 text-center",
        STYLE[style],
        face?.highlighted && "ring-2 ring-sky-400 ring-inset",
        className,
      )}
    >
      {style !== "empty" &&
        !compact &&
        (Glyph ? (
          <Glyph className="size-4 shrink-0 opacity-80" aria-hidden />
        ) : style === "action" ? (
          <ActionIcon name={icon} className="size-4 shrink-0 opacity-80" />
        ) : null)}
      {face?.title && (
        <span
          className={cn(
            "text-[10px] leading-tight font-medium break-words",
            compact ? "w-full truncate" : "line-clamp-2",
          )}
        >
          {face.title}
        </span>
      )}
      {face?.detail && <span className="text-[9px] leading-none text-sky-300">{face.detail}</span>}
    </div>
  )
}
