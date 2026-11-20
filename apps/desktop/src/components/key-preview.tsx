import { useEffect, useState } from "react"

import { FaceTile } from "@/components/key-face"
import type { KeyFace, MarqueePreview } from "@/lib/types"

/**
 * A key as the device draws it: the renderer's image of the key (the same
 * image, at the same size, the Device Canvas shows), its scrolling title's
 * frames played in a loop, or until the image arrives the drawn face. The
 * one key preview of the app: the Canvas and the Inspector's key header
 * show it, so both always match the hardware.
 */
export function KeyPreview({
  face,
  image,
  animation,
  icon,
  compact = false,
  unavailable,
}: {
  face: KeyFace | undefined
  /** The rendered image (PNG data URL); shown instead of the drawn face. */
  image?: string
  animation?: MarqueePreview
  /** The action's icon by name, for the drawn face. */
  icon?: string | null
  /** Text only, for short regions such as touch-strip zones. */
  compact?: boolean
  /** Why the key cannot do anything now: drawn dimmed with the reason. */
  unavailable?: string
}) {
  if (image && unavailable === undefined && face?.style !== "empty") {
    return <KeyImage image={image} animation={animation} />
  }
  return (
    <FaceTile
      face={
        unavailable === undefined
          ? face
          : { ...(face ?? { style: "action", title: "" }), detail: unavailable }
      }
      icon={icon}
      compact={compact}
      unavailable={unavailable !== undefined}
    />
  )
}

/**
 * A key's rendered image, or the frames of its scrolling title played in a
 * loop (each for its time, as on the device).
 */
export function KeyImage({ image, animation }: { image: string; animation?: MarqueePreview }) {
  // The frame shown, for the animation it belongs to.
  const [shown, setShown] = useState<{ animation: MarqueePreview; index: number } | null>(null)
  useEffect(() => {
    if (!animation || animation.frames.length < 2) return
    let index = 0
    let timer: ReturnType<typeof setTimeout>
    const advance = () => {
      timer = setTimeout(() => {
        index = (index + 1) % animation.frames.length
        setShown({ animation, index })
        advance()
      }, animation.frames[index]?.durationMs ?? 100)
    }
    advance()
    return () => {
      clearTimeout(timer)
    }
  }, [animation])
  const frame = shown && shown.animation === animation ? shown.index : 0
  const src = animation?.frames[frame]?.image ?? animation?.frames[0]?.image ?? image
  return (
    <img
      src={src}
      alt=""
      draggable={false}
      data-slot="key-image"
      data-scrolling={animation ? "" : undefined}
      // Fills the area inside the border exactly (an aspect-ratio box
      // gives no definite height for a percentage to resolve against).
      className="absolute inset-0 size-full object-cover"
    />
  )
}
