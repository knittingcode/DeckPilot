import { ArrowLeft, Folder, ListOrdered } from "lucide-react"

import { ActionIcon } from "@/components/action-icon"
import type { KeyFace } from "@/lib/types"
import { cn } from "@/lib/utils"

// Faces drawn until the device's own image of the key arrives: neutral LCD
// tones from the device tokens, the key's icon first and its title under it.
const STYLE: Record<KeyFace["style"], string> = {
  empty: "bg-transparent",
  action: "bg-device-face text-device-key-foreground",
  workflow: "bg-device-face text-device-key-foreground",
  folder: "bg-device-face-folder text-device-key-foreground",
  back: "bg-device-face text-device-key-foreground",
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
  // Sized from the key (`--key` of the device canvas), with a floor for
  // the smallest previews.
  const glyph = "size-[max(14px,calc(var(--key,4rem)*0.34))] shrink-0 opacity-90"
  return (
    <div
      data-slot="key-face"
      data-face={style}
      className={cn(
        "flex size-full flex-col items-center justify-center gap-[calc(var(--key,4rem)*0.05)] overflow-hidden p-[calc(var(--key,4rem)*0.08)] text-center",
        STYLE[style],
        face?.highlighted && "ring-2 ring-sky-400 ring-inset",
        className,
      )}
    >
      {style !== "empty" &&
        !compact &&
        (Glyph ? (
          <Glyph className={glyph} aria-hidden />
        ) : style === "action" ? (
          <ActionIcon name={icon} className={glyph} />
        ) : null)}
      {face?.title && (
        <span
          className={cn(
            "max-w-full text-[length:max(9px,calc(var(--key,4rem)*0.13))] leading-tight font-medium break-words",
            compact ? "w-full truncate" : "line-clamp-2 text-device-key-muted",
          )}
        >
          {face.title}
        </span>
      )}
      {face?.detail && (
        <span className="text-[length:max(8px,calc(var(--key,4rem)*0.11))] leading-none text-sky-300">
          {face.detail}
        </span>
      )}
    </div>
  )
}
