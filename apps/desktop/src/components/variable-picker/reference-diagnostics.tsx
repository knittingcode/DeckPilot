import { CircleAlert, Info } from "lucide-react"

import type { ParameterKind } from "@/lib/types"
import { cn } from "@/lib/utils"
import { describeContext, describeStatus, formatReference, formatValue } from "@/lib/variables"

import { useVariableScope } from "./scope"
import { useReferencePreviews } from "./use-variables"

/**
 * The live state of the references a field uses: the current value, or
 * why it cannot be resolved (missing, wrong type, denied, invalid), in the
 * scope's context. Sensitive values only ever appear as "redacted".
 */
export function ReferenceDiagnostics({
  references,
  kind,
  unclosed = false,
  literalBraces = false,
}: {
  references: string[]
  /** What each reference must fill: the field's kind, or text in a template. */
  kind: ParameterKind
  /** A `{{` without `}}`. */
  unclosed?: boolean
  /** Plain text that contains `{{`, which stays literal text. */
  literalBraces?: boolean
}) {
  const scope = useVariableScope()
  const previews = useReferencePreviews(
    scope ?? FALLBACK,
    references.map((reference) => ({ reference, expected: kind })),
    Boolean(scope),
  )
  if (!scope || (references.length === 0 && !unclosed && !literalBraces)) return null
  return (
    <ul className="grid gap-0.5 text-xs" aria-label="References">
      {unclosed && <Line tone="error">A reference is not closed: add “{"}}"}” after it.</Line>}
      {literalBraces && (
        <Line tone="info">
          “{"{{"}” in plain text stays as typed. Insert a variable to use a reference.
        </Line>
      )}
      {references.map((reference) => {
        const status = previews.status(reference)
        const label = <code className="font-mono">{formatReference(reference)}</code>
        if (!scope.live) {
          return (
            <Line key={reference} tone="error">
              {label} cannot be resolved by this DeckPilot agent; update the agent or use a fixed
              value.
            </Line>
          )
        }
        if (previews.error) {
          return (
            <Line key={reference} tone="error">
              {label}: {previews.error}
            </Line>
          )
        }
        if (!status) {
          return (
            <Line key={reference} tone="info">
              {label}: resolving…
            </Line>
          )
        }
        if (status.status === "resolved") {
          return (
            <Line key={reference} tone="info">
              {label} is now <span className="font-mono">{formatValue(status.value)}</span>
            </Line>
          )
        }
        const description = describeStatus(status)
        return (
          <Line key={reference} tone={description.tone === "error" ? "error" : "info"}>
            {label}: {description.label}
            {description.detail && ` — ${description.detail}`}
          </Line>
        )
      })}
      {scope.live && references.length > 0 && (
        <li className="text-muted-foreground">
          Values for {describeContext(scope.context, scope.names)}
        </li>
      )}
    </ul>
  )
}

const FALLBACK = {
  context: { profile: "" },
  names: {},
  live: false,
  stepOutputs: [],
}

function Line({ tone, children }: { tone: "info" | "error"; children: React.ReactNode }) {
  const Icon = tone === "error" ? CircleAlert : Info
  return (
    <li
      className={cn(
        "flex gap-1 break-words",
        tone === "error" ? "text-destructive" : "text-muted-foreground",
      )}
    >
      <Icon className="mt-0.5 size-3.5 shrink-0" aria-hidden />
      <span className="min-w-0">{children}</span>
    </li>
  )
}
