import { CircleAlert } from "lucide-react"

import { cn } from "@/lib/utils"

/**
 * The inline validation message of a form control, shown directly below
 * it: a small warning icon and short text, no box. The control points at
 * it with `aria-describedby={id}` and sets `aria-invalid` while it shows.
 *
 * `reserve` keeps one line of space while there is no message, so a
 * window or dialog does not change height when validation fails. The
 * message is an alert: it is announced when it appears, and again after
 * each new attempt (callers clear it while an attempt runs).
 */
export function FieldError({
  id,
  message,
  reserve = false,
  className,
}: {
  id: string
  message?: string | null
  reserve?: boolean
  className?: string
}) {
  if (!message && !reserve) return null
  return (
    <div className={cn("min-h-4", className)}>
      {message && (
        <p
          id={id}
          role="alert"
          className="flex items-start gap-1.5 text-xs leading-4 text-destructive"
        >
          <CircleAlert className="mt-px size-3.5 shrink-0" aria-hidden />
          <span>{message}</span>
        </p>
      )}
    </div>
  )
}
