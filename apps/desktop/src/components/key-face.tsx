import { ArrowLeft, Folder, ListOrdered } from "lucide-react"

import { ActionIcon } from "@/components/action-icon"
import type { KeyFace } from "@/lib/types"
import { cn } from "@/lib/utils"
// The folder's system Back icon: one core-owned file, the one the agent's
// renderer draws (crates/deckpilot-render/assets/icons/folder-back.svg).
import folderBackIcon from "../../../../crates/deckpilot-render/assets/icons/folder-back.svg?url"

/** The `symbol` of the folder's system Back face. */
export const FOLDER_BACK_SYMBOL = "folder-back"

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
  unavailable = false,
  className,
}: {
  face: KeyFace | undefined
  icon?: string | null
  /** Text only, for short regions such as touch-strip zones. */
  compact?: boolean
  /** It cannot do anything now (e.g. Back with nowhere to go): dimmed. */
  unavailable?: boolean
  className?: string
}) {
  const style = face?.style ?? "empty"
  const folderBack = face?.symbol === FOLDER_BACK_SYMBOL
  const Glyph = folderBack
    ? null
    : style === "folder"
      ? Folder
      : style === "back"
        ? ArrowLeft
        : style === "workflow"
          ? ListOrdered
          : null
  // Sized from the key (`--key` of the device canvas) in the proportions
  // the device's renderer uses (deckpilot-render): the icon about two
  // fifths of the key above a title, a little over half without one; a
  // floor for the smallest previews.
  const titled = Boolean(face?.title || face?.detail)
  const glyph = cn(
    "shrink-0",
    titled
      ? "size-[max(14px,calc(var(--key,4rem)*0.42))]"
      : "size-[max(16px,calc(var(--key,4rem)*0.54))]",
  )
  return (
    <div
      data-slot="key-face"
      data-face={style}
      data-unavailable={unavailable || undefined}
      className={cn(
        "flex size-full min-w-0 flex-col items-center justify-center gap-[calc(var(--key,4rem)*0.06)] overflow-hidden p-[calc(var(--key,4rem)*0.07)] text-center",
        STYLE[style],
        // As the device draws it: white on black.
        folderBack && "bg-black text-white",
        face?.highlighted && "ring-2 ring-sky-400 ring-inset",
        unavailable && "opacity-50",
        className,
      )}
    >
      {style !== "empty" &&
        !compact &&
        (folderBack ? (
          // The file itself, drawn white through a mask (not redrawn here).
          <span
            aria-hidden
            data-slot="folder-back-icon"
            className={cn(glyph, "bg-white")}
            style={{
              maskImage: `url("${folderBackIcon}")`,
              WebkitMaskImage: `url("${folderBackIcon}")`,
              maskRepeat: "no-repeat",
              maskPosition: "center",
              maskSize: "contain",
            }}
          />
        ) : Glyph ? (
          <Glyph className={glyph} aria-hidden />
        ) : style === "action" ? (
          <ActionIcon name={icon} className={glyph} />
        ) : null)}
      {face?.title && !folderBack && (
        <span
          className={cn(
            "max-w-full text-[length:max(9px,calc(var(--key,4rem)*0.13))] leading-[1.2] font-medium [overflow-wrap:anywhere]",
            compact ? "w-full truncate" : "line-clamp-2",
          )}
        >
          {face.title}
        </span>
      )}
      {face?.detail && (
        <span
          className={cn(
            "max-w-full truncate text-[length:max(8px,calc(var(--key,4rem)*0.13))] leading-[1.2] font-semibold",
            face.highlighted ? "text-sky-300" : "text-device-key-muted",
          )}
        >
          {face.detail}
        </span>
      )}
    </div>
  )
}
