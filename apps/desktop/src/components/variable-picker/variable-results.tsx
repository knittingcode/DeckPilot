import { Lock } from "lucide-react"

import type { CatalogEntry, ParameterKind, ReferenceStatus } from "@/lib/types"
import { cn } from "@/lib/utils"
import {
  AVAILABILITY_LABEL,
  SCOPE_LABEL,
  type StatusTone,
  describeContext,
  describeSource,
  describeStatus,
  formatReference,
  formatValue,
  typeLabel,
} from "@/lib/variables"

import type { VariableScope } from "./scope"
import {
  type BrowserState,
  type PreviewState,
  optionId,
  useReferencePreviews,
} from "./use-variables"

/**
 * The grouped list of references (listbox) and the inspector of the active
 * one. Focus stays in the search or text field that owns the list; the
 * field points at the active option with `aria-activedescendant`.
 */
export function VariableResults({
  listId,
  scope,
  kind,
  browser,
  onPick,
}: {
  listId: string
  scope: VariableScope
  kind: ParameterKind | undefined
  browser: BrowserState
  onPick: (entry: CatalogEntry) => void
}) {
  const { catalog, groups, ordered, active } = browser
  const previews = useReferencePreviews(
    scope,
    active ? [{ reference: active.reference, expected: kind }] : [],
  )
  return (
    <div className="grid min-h-0 grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-0 divide-x">
      <div className="max-h-72 min-h-24 overflow-y-auto p-1">
        {catalog.unavailable && (
          <p className="px-2 py-1 text-xs text-muted-foreground" role="status">
            Values are not available: {catalog.unavailable}
          </p>
        )}
        <div id={listId} role="listbox" aria-label="Variables" className="grid gap-1">
          {groups.map((group) => {
            const headingId = `${listId}-${group.group}`
            return (
              <div key={group.group} role="group" aria-labelledby={headingId}>
                <div
                  id={headingId}
                  className="px-2 pt-1 text-[0.65rem] font-medium text-muted-foreground uppercase"
                >
                  {group.label}
                </div>
                {group.entries.map((entry) => {
                  const index = ordered.indexOf(entry)
                  const selected = entry === active
                  return (
                    <div
                      key={entry.reference}
                      id={optionId(listId, index)}
                      role="option"
                      aria-selected={selected}
                      className={cn(
                        "grid cursor-default gap-0.5 rounded-sm px-2 py-1 text-xs",
                        selected && "bg-accent text-accent-foreground",
                        entry.availability === "notInContext" && "opacity-60",
                      )}
                      // Keep the focus in the field that owns the list.
                      onMouseDown={(e) => {
                        e.preventDefault()
                      }}
                      onMouseMove={() => {
                        if (!selected) browser.setActive(entry.reference)
                      }}
                      onClick={() => {
                        onPick(entry)
                      }}
                    >
                      <span className="flex items-center gap-1 font-medium">
                        {entry.label}
                        {entry.sensitive && (
                          <Lock className="size-3 text-muted-foreground" aria-label="sensitive" />
                        )}
                        <span className="ml-auto font-normal text-muted-foreground">
                          {typeLabel(entry.valueType)}
                        </span>
                      </span>
                      <code className="truncate text-[0.7rem] text-muted-foreground">
                        {entry.reference}
                      </code>
                    </div>
                  )
                })}
              </div>
            )
          })}
        </div>
        {!catalog.loading && ordered.length === 0 && (
          <p className="px-2 py-3 text-xs text-muted-foreground" role="status">
            {kind ? "No variable fits this field." : "No variables match."}
          </p>
        )}
        {catalog.loading && ordered.length === 0 && (
          <p className="px-2 py-3 text-xs text-muted-foreground">Loading variables…</p>
        )}
      </div>
      <div className="max-h-72 overflow-y-auto p-2">
        {active ? (
          <VariableInspector entry={active} scope={scope} previews={previews} />
        ) : (
          <p className="text-xs text-muted-foreground">Choose a variable to see its value.</p>
        )}
      </div>
    </div>
  )
}

const TONE: Record<StatusTone, string> = {
  ok: "text-foreground",
  info: "text-muted-foreground",
  error: "text-destructive",
}

/**
 * The value cell of the inspector. A sensitive entry never shows a value,
 * whatever the resolver returned.
 */
export function PreviewValue({
  status,
  sensitive,
  live,
  loading,
  error,
}: {
  status: ReferenceStatus | undefined
  sensitive?: boolean
  live: boolean
  loading: boolean
  error?: string
}) {
  if (!live) return <span className="text-muted-foreground">Not available from this agent</span>
  if (error) return <span className="text-destructive">Could not be resolved: {error}</span>
  if (!status) {
    return <span className="text-muted-foreground">{loading ? "Resolving…" : "Unknown"}</span>
  }
  if (sensitive || status.status === "redacted") {
    return <span className="text-muted-foreground italic">Redacted (sensitive)</span>
  }
  if (status.status === "resolved") {
    return <span className="font-mono break-all">{formatValue(status.value)}</span>
  }
  const description = describeStatus(status)
  return (
    <span className={TONE[description.tone]}>
      {description.label}
      {description.detail && `: ${description.detail}`}
    </span>
  )
}

/** Everything known about one reference, with its current value. */
export function VariableInspector({
  entry,
  scope,
  previews,
}: {
  entry: CatalogEntry
  scope: VariableScope
  previews: PreviewState
}) {
  const status = previews.status(entry.reference)
  const source =
    status && (status.status === "resolved" || status.status === "redacted")
      ? describeSource(status.source)
      : null
  return (
    <section aria-label="Variable details" className="grid gap-2 text-xs">
      <div>
        <h3 className="text-sm font-medium">{entry.label}</h3>
        <code className="break-all text-muted-foreground">{formatReference(entry.reference)}</code>
      </div>
      {entry.description && <p>{entry.description}</p>}
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-2 gap-y-1">
        <dt className="text-muted-foreground">Value</dt>
        <dd aria-live="polite" aria-label="Current value">
          <PreviewValue
            status={status}
            sensitive={entry.sensitive}
            live={scope.live}
            loading={previews.loading}
            error={previews.error}
          />
        </dd>
        {source && (
          <>
            <dt className="text-muted-foreground">Source</dt>
            <dd>{source}</dd>
          </>
        )}
        <dt className="text-muted-foreground">Type</dt>
        <dd>{typeLabel(entry.valueType)}</dd>
        {entry.scope && (
          <>
            <dt className="text-muted-foreground">Scope</dt>
            <dd>{SCOPE_LABEL[entry.scope]}</dd>
          </>
        )}
        {entry.module && (
          <>
            <dt className="text-muted-foreground">Module</dt>
            <dd>{entry.module}</dd>
          </>
        )}
        <dt className="text-muted-foreground">Changes</dt>
        <dd>{entry.dynamic ? "Dynamic: depends on the context" : "Static"}</dd>
        <dt className="text-muted-foreground">Availability</dt>
        <dd>{AVAILABILITY_LABEL[entry.availability]}</dd>
        {entry.sensitive && (
          <>
            <dt className="text-muted-foreground">Sensitive</dt>
            <dd>Never shown, logged or exported</dd>
          </>
        )}
      </dl>
      <p className="text-muted-foreground" aria-label="Resolution context">
        Resolved for: {describeContext(scope.context, scope.names)}
      </p>
    </section>
  )
}
