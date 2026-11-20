import { useLayoutEffect, useRef, useState, type ComponentProps } from "react"

import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"

/** Room for the caret after the last character, in pixels. */
const CARET = 2

/**
 * A text field as wide as its value: the shared `Input`, measured with a
 * hidden mirror of the same text in the same font (the same classes, so the
 * same size, weight, letter spacing and padding) before every paint, so it
 * grows and shrinks while typing without a frame at the wrong width.
 * Between `minWidth` and `maxWidth` (CSS pixels); beyond the maximum the
 * text scrolls inside the field (the caret stays visible), it is never cut
 * or ellipsized.
 */
export function AutoSizeInput({
  value,
  minWidth = 64,
  maxWidth = Number.POSITIVE_INFINITY,
  className,
  style,
  ref,
  ...props
}: Omit<ComponentProps<"input">, "value"> & {
  value: string
  minWidth?: number
  maxWidth?: number
}) {
  const mirror = useRef<HTMLSpanElement>(null)
  const [width, setWidth] = useState<number | null>(null)

  useLayoutEffect(() => {
    const element = mirror.current
    if (!element) return
    // The mirror has the field's padding and border: its width is the
    // field's width for this text.
    const measured = Math.ceil(element.getBoundingClientRect().width) + CARET
    const upper = Math.max(minWidth, maxWidth)
    setWidth(Math.min(Math.max(measured, minWidth), upper))
  }, [value, minWidth, maxWidth])

  const shared = cn("h-7 px-1.5 py-0 text-sm md:text-sm", className)
  return (
    <span className="relative inline-flex max-w-full" data-slot="auto-size-input">
      <span
        ref={mirror}
        aria-hidden
        data-slot="auto-size-mirror"
        className={cn(
          shared,
          "pointer-events-none invisible absolute top-0 left-0 inline-block w-auto border border-transparent whitespace-pre",
        )}
      >
        {value || " "}
      </span>
      <Input
        ref={ref}
        value={value}
        className={cn(shared, "shadow-none")}
        style={{ ...style, width: width ?? minWidth }}
        {...props}
      />
    </span>
  )
}
