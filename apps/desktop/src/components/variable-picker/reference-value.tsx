import { Braces } from "lucide-react"

import { Button } from "@/components/ui/button"
import type { ParameterKind } from "@/lib/types"
import { formatReference } from "@/lib/variables"

import { ReferenceDiagnostics } from "./reference-diagnostics"
import { VariablePicker } from "./variable-picker"

/**
 * A field whose whole value is a reference: shows the reference (never a
 * previewed value, which is not what is saved), its live state, and lets
 * the user choose another variable or go back to a fixed value.
 */
export function ReferenceValueField({
  id,
  labelId,
  fieldLabel,
  reference,
  kind,
  onReference,
  onFixedValue,
}: {
  id: string
  labelId: string
  fieldLabel: string
  reference: string
  kind: ParameterKind
  onReference: (reference: string) => void
  onFixedValue: () => void
}) {
  return (
    <div className="grid gap-1">
      <div
        role="group"
        aria-labelledby={labelId}
        className="flex min-w-0 items-center gap-2 rounded-md border px-2 py-1"
      >
        <Braces className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        <code id={id} className="min-w-0 flex-1 truncate text-sm" aria-label="Reference">
          {formatReference(reference)}
        </code>
        <VariablePicker
          kind={kind}
          fieldLabel={fieldLabel}
          label={`Choose another variable for ${fieldLabel}`}
          className="size-7"
          onPick={onReference}
        />
        <Button type="button" variant="ghost" size="sm" className="h-7" onClick={onFixedValue}>
          Use a fixed value
        </Button>
      </div>
      <ReferenceDiagnostics references={[reference]} kind={kind} />
    </div>
  )
}
