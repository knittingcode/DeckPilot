import { useId, useLayoutEffect, useRef, useState } from "react"

import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import type { ParameterKind } from "@/lib/types"
import { cn } from "@/lib/utils"
import { TEMPLATE_KIND, autocompleteAt, insertReference } from "@/lib/variables"

import { useVariableScope } from "./scope"
import { VariablePicker } from "./variable-picker"
import { VariableResults } from "./variable-results"
import { activeDescendant, useVariableBrowser } from "./use-variables"

type Field = HTMLInputElement | HTMLTextAreaElement

/**
 * A text field that takes `{{ … }}` references: typing `{{` opens the
 * picker's list under the field (arrows, Enter, Escape), and the button
 * beside it inserts at the cursor. `onChange` says whether the change came
 * from inserting a reference. Without an agent that resolves variables it
 * is a plain text field.
 */
export function ReferenceInput({
  id,
  value,
  onChange,
  fieldLabel,
  kind = TEMPLATE_KIND,
  multiline = false,
  maxLength,
  placeholder,
  className,
  "aria-label": ariaLabel,
  "aria-invalid": ariaInvalid,
}: {
  id?: string
  value: string
  onChange: (text: string, inserted: boolean) => void
  /** Names the field in the picker button's label. */
  fieldLabel: string
  /** What references must fill; text for a template. */
  kind?: ParameterKind
  multiline?: boolean
  maxLength?: number
  placeholder?: string
  className?: string
  "aria-label"?: string
  "aria-invalid"?: boolean
}) {
  const scope = useVariableScope()
  const listId = useId()
  const field = useRef<Field>(null)
  const [cursor, setCursor] = useState<number | null>(null)
  const [focused, setFocused] = useState(false)
  // The `{{` the user closed the list for with Escape.
  const [dismissed, setDismissed] = useState<number | null>(null)
  const pendingCursor = useRef<number | null>(null)

  const live = Boolean(scope?.live)
  const autocomplete = live && focused && cursor !== null ? autocompleteAt(value, cursor) : null
  const open = autocomplete !== null && autocomplete.start !== dismissed
  const browser = useVariableBrowser(
    scope ?? { context: { profile: "" }, names: {}, live: false, stepOutputs: [] },
    autocomplete?.query ?? "",
    kind,
    open,
  )

  useLayoutEffect(() => {
    const at = pendingCursor.current
    if (at === null || !field.current) return
    pendingCursor.current = null
    field.current.focus()
    field.current.setSelectionRange(at, at)
  }, [value])

  const insert = (reference: string, start: number, end: number) => {
    const next = insertReference(value, start, end, reference)
    pendingCursor.current = next.cursor
    setCursor(next.cursor)
    setDismissed(null)
    onChange(next.text, true)
  }

  const track = (element: Field) => {
    setCursor(element.selectionStart)
  }

  const props = {
    id,
    ref: (element: Field | null) => {
      field.current = element
    },
    value,
    maxLength,
    placeholder,
    "aria-label": ariaLabel,
    "aria-invalid": ariaInvalid,
    className: cn("min-w-0 flex-1", className),
    onChange: (e: React.ChangeEvent<Field>) => {
      setCursor(e.target.selectionStart)
      onChange(e.target.value, false)
    },
    onSelect: (e: React.SyntheticEvent<Field>) => {
      track(e.currentTarget)
    },
    onFocus: (e: React.FocusEvent<Field>) => {
      setFocused(true)
      track(e.currentTarget)
    },
    onBlur: () => {
      setFocused(false)
    },
    ...(live && {
      role: "combobox",
      "aria-autocomplete": "list" as const,
      "aria-expanded": open,
      "aria-controls": open ? listId : undefined,
      "aria-activedescendant": open ? activeDescendant(listId, browser) : undefined,
      onKeyDown: (e: React.KeyboardEvent<Field>) => {
        if (!open) return
        if (e.key === "Escape") {
          e.preventDefault()
          e.stopPropagation()
          setDismissed(autocomplete.start)
          return
        }
        // Home and End keep moving the cursor in the text.
        if (e.key === "Home" || e.key === "End") return
        browser.handleKey(e, (entry) => {
          insert(entry.reference, autocomplete.start, cursor ?? value.length)
        })
      },
    }),
  }

  return (
    <div className="relative grid gap-1">
      <div className={cn("flex gap-1", multiline && "items-start")}>
        {multiline ? <Textarea {...props} /> : <Input {...props} />}
        <VariablePicker
          kind={kind}
          fieldLabel={fieldLabel}
          onPick={(reference) => {
            const element = field.current
            const start = element?.selectionStart ?? value.length
            const end = element?.selectionEnd ?? start
            insert(reference, start, end)
          }}
        />
      </div>
      {open && scope && (
        <div
          className="absolute top-full right-0 left-0 z-50 mt-1 rounded-md border bg-popover text-popover-foreground shadow-md"
          // Clicks in the list must not take the focus from the field.
          onMouseDown={(e) => {
            e.preventDefault()
          }}
        >
          <VariableResults
            listId={listId}
            scope={scope}
            kind={kind}
            browser={browser}
            onPick={(entry) => {
              insert(entry.reference, autocomplete.start, cursor ?? value.length)
            }}
          />
        </div>
      )}
    </div>
  )
}
