import { useId, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"

import { SWATCHES, type Shared, parseHex } from "./appearance"

/**
 * A key colour: swatches, a hex field and a reset to the style's colour.
 * Typing in the hex field forms one undo step (`group`).
 */
export function ColorField({
  label,
  value,
  onChange,
}: {
  label: string
  value: Shared<string>
  onChange: (value: string | null, group?: string) => void
}) {
  const id = useId()
  const current = value === "mixed" ? undefined : value
  // What was typed, and the value it was typed against. An outside change
  // (undo, another key selected) replaces the text.
  const [typed, setTyped] = useState({ text: current ?? "", base: current })
  const text = typed.base === current ? typed.text : (current ?? "")
  const invalid = text.trim() !== "" && parseHex(text) === null

  return (
    <fieldset className="grid gap-2">
      <legend className="mb-1 text-sm font-medium">{label}</legend>
      <div className="flex flex-wrap gap-1" role="group" aria-label={`${label} swatches`}>
        {SWATCHES.map((swatch) => (
          <button
            key={swatch.value}
            type="button"
            aria-label={swatch.name}
            aria-pressed={current === swatch.value}
            title={`${swatch.name} ${swatch.value}`}
            className={cn(
              "size-6 rounded-md border focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
              current === swatch.value && "ring-2 ring-primary ring-offset-1",
            )}
            style={{ backgroundColor: swatch.value }}
            onClick={() => {
              onChange(swatch.value)
            }}
          />
        ))}
      </div>
      <div className="flex items-center gap-2">
        <input
          type="color"
          aria-label={`${label} picker`}
          className="h-8 w-10 cursor-pointer rounded border bg-transparent p-0.5"
          value={current ?? "#000000"}
          onChange={(e) => {
            onChange(e.target.value, `appearance:${label}`)
          }}
        />
        <Input
          id={id}
          aria-label={`${label} hex`}
          aria-invalid={invalid}
          aria-describedby={invalid ? `${id}-error` : undefined}
          className="h-8 w-28 font-mono"
          placeholder={value === "mixed" ? "Mixed" : "Default"}
          value={text}
          maxLength={7}
          onChange={(e) => {
            const next = e.target.value
            const parsed = parseHex(next)
            const empty = next.trim() === ""
            setTyped({ text: next, base: parsed ?? (empty ? undefined : current) })
            if (parsed) onChange(parsed, `appearance:${label}`)
            else if (empty) onChange(null, `appearance:${label}`)
          }}
        />
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={value === undefined}
          onClick={() => {
            onChange(null)
          }}
        >
          Default
        </Button>
      </div>
      {invalid && (
        <p id={`${id}-error`} className="text-xs text-destructive">
          Enter a colour as #rrggbb.
        </p>
      )}
    </fieldset>
  )
}
