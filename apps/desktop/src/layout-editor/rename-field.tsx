import { useEffect, useId, useRef, useState, type CSSProperties } from "react"
import { createPortal } from "react-dom"

import { AutoSizeInput } from "@/components/auto-size-input"
import { FieldError } from "@/components/dialogs"
import { useTranslation } from "@/i18n"
import { errorMessage } from "@/lib/api"
import { cn } from "@/lib/utils"

/**
 * A page name edited in place (the breadcrumb, the Page Navigator): Enter
 * or leaving the field keeps a valid name, Escape cancels; an empty name is
 * refused and editing goes on. `onDone` gets the new name, or `null` when
 * nothing is to be written (cancelled or unchanged), exactly once; if it
 * throws, the typed name stays with why. Keys typed here reach nothing
 * else (the editor's shortcuts, the canvas, the tree).
 *
 * With `fit`, the field is as wide as its text (`AutoSizeInput`) up to
 * `fit.maxWidth`; otherwise it fills the space it is given.
 */
export function RenameField({
  initial,
  onDone,
  className,
  style,
  label,
  fit,
}: {
  initial: string
  onDone: (name: string | null) => void
  className?: string
  style?: CSSProperties
  label?: string
  fit?: { minWidth: number; maxWidth: number }
}) {
  const { t } = useTranslation()
  const errorId = useId()
  const [value, setValue] = useState(initial)
  const [error, setError] = useState<{ text: string; left: number; top: number } | null>(null)
  const done = useRef(false)
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => {
    input.current?.focus()
    input.current?.select()
  }, [])
  const fail = (text: string) => {
    const box = input.current?.getBoundingClientRect()
    setError({ text, left: box?.left ?? 0, top: (box?.bottom ?? 0) + 4 })
    input.current?.focus()
  }
  const finish = (commit: boolean) => {
    if (done.current) return
    if (!commit) {
      done.current = true
      onDone(null)
      return
    }
    const name = value.trim()
    if (!name) {
      fail(t("editor:navigator.emptyName"))
      return
    }
    done.current = true
    try {
      onDone(name === initial.trim() ? null : name)
    } catch (e) {
      // Not renamed: the typed name stays, with why.
      done.current = false
      fail(t("editor:navigator.renameFailed", { reason: errorMessage(e) }))
    }
  }
  const props = {
    ref: input,
    value,
    "aria-label": label ?? t("editor:pageSettings.name"),
    "aria-invalid": error !== null || undefined,
    "aria-describedby": error ? errorId : undefined,
    title: t("editor:navigator.renameHint"),
    "data-slot": "rename-field",
    onChange: (event: React.ChangeEvent<HTMLInputElement>) => {
      setValue(event.target.value)
      setError(null)
    },
    onClick: (event: React.MouseEvent) => {
      event.stopPropagation()
    },
    onDoubleClick: (event: React.MouseEvent) => {
      event.stopPropagation()
    },
    onPointerDown: (event: React.PointerEvent) => {
      event.stopPropagation()
    },
    onKeyDown: (event: React.KeyboardEvent<HTMLInputElement>) => {
      // Nothing of the editor, the canvas or the tree reacts to typing here.
      event.stopPropagation()
      if (event.key === "Enter") {
        event.preventDefault()
        finish(true)
      } else if (event.key === "Escape") {
        event.preventDefault()
        finish(false)
      }
    },
    onBlur: () => {
      finish(true)
    },
  }
  return (
    <span className={cn("relative min-w-0", !fit && "flex-1", className)} style={style}>
      {fit ? (
        <AutoSizeInput
          {...props}
          minWidth={fit.minWidth}
          maxWidth={fit.maxWidth}
          // The breadcrumb it replaces: the navigation controls' height and padding.
          className="h-8 px-2.5 font-medium"
        />
      ) : (
        <input
          {...props}
          className="h-6 w-full min-w-0 rounded-sm border border-input bg-background px-1.5 text-sm text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring aria-invalid:border-destructive"
        />
      )}
      {error &&
        // Under the field, outside any clipping container (the breadcrumbs clip).
        createPortal(
          <span
            data-slot="rename-error"
            className="fixed z-50 w-max max-w-64 rounded-sm border bg-popover px-1.5 py-0.5 shadow-sm"
            style={{ left: error.left, top: error.top }}
          >
            <FieldError id={errorId} message={error.text} />
          </span>,
          document.body,
        )}
    </span>
  )
}
