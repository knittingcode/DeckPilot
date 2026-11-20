import { Lock } from "lucide-react"

import { Trans, useTranslation } from "@/i18n"
import type { CatalogEntry, ParameterKind, ReferenceStatus } from "@/lib/types"
import { cn } from "@/lib/utils"
import { ownerLabel, ownerOf } from "@/lib/variable-catalog"
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
  const { t } = useTranslation()
  const { catalog, groups, ordered, active } = browser
  const previews = useReferencePreviews(
    scope,
    active ? [{ reference: active.reference, expected: kind }] : [],
  )
  return (
    <div className="grid min-h-0 grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-0 divide-x">
      <div className="relative max-h-72 min-h-24 overflow-y-auto p-1">
        {catalog.unavailable && (
          <p className="px-2 py-1 text-xs text-muted-foreground" role="status">
            {t("editor:variables.unavailable", { reason: catalog.unavailable })}
          </p>
        )}
        <div
          id={listId}
          role="listbox"
          aria-label={t("editor:variables.list")}
          className="grid gap-1"
        >
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
                        (entry.availability === "notInContext" ||
                          entry.availability === "unavailable") &&
                          "opacity-60",
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
                          <Lock
                            className="size-3 text-muted-foreground"
                            aria-label={t("editor:variables.sensitiveMark")}
                          />
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
            {kind ? t("editor:variables.noneFits") : t("editor:variables.noMatches")}
          </p>
        )}
        {catalog.loading && ordered.length === 0 && (
          <p className="px-2 py-3 text-xs text-muted-foreground">{t("editor:variables.loading")}</p>
        )}
      </div>
      <div className="relative max-h-72 overflow-y-auto p-2">
        {active ? (
          <VariableInspector entry={active} scope={scope} previews={previews} />
        ) : (
          <p className="text-xs text-muted-foreground">{t("editor:variables.chooseToSee")}</p>
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
  const { t } = useTranslation()
  if (!live) {
    return <span className="text-muted-foreground">{t("editor:variables.preview.notLive")}</span>
  }
  if (error) {
    return (
      <span className="text-destructive">
        {t("editor:variables.preview.failed", { reason: error })}
      </span>
    )
  }
  if (!status) {
    return (
      <span className="text-muted-foreground">
        {loading ? t("editor:variables.resolving") : t("editor:variables.preview.unknown")}
      </span>
    )
  }
  if (sensitive || status.status === "redacted") {
    return (
      <span className="text-muted-foreground italic">{t("editor:variables.preview.redacted")}</span>
    )
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
  const { t } = useTranslation()
  const status = previews.status(entry.reference)
  const source =
    status && (status.status === "resolved" || status.status === "redacted")
      ? describeSource(status.source)
      : null
  return (
    <section aria-label={t("editor:variables.details.label")} className="grid gap-2 text-xs">
      <div>
        <h3 className="text-sm font-medium">{entry.label}</h3>
        <code className="break-all text-muted-foreground">{formatReference(entry.reference)}</code>
      </div>
      {entry.description && <p>{entry.description}</p>}
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-2 gap-y-1">
        <dt className="text-muted-foreground">{t("editor:variables.details.value")}</dt>
        <dd aria-live="polite" aria-label={t("editor:variables.details.currentValue")}>
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
            <dt className="text-muted-foreground">{t("editor:variables.details.source")}</dt>
            <dd>{source}</dd>
          </>
        )}
        <dt className="text-muted-foreground">{t("editor:variables.details.type")}</dt>
        <dd>{typeLabel(entry.valueType)}</dd>
        {entry.scope && (
          <>
            <dt className="text-muted-foreground">{t("editor:variables.details.scope")}</dt>
            <dd>{t(SCOPE_LABEL[entry.scope])}</dd>
          </>
        )}
        <dt className="text-muted-foreground">{t("editor:variables.details.owner")}</dt>
        <dd>{ownerLabel(ownerOf(entry))}</dd>
        <dt className="text-muted-foreground">{t("editor:variables.details.access")}</dt>
        <dd>
          {ownerOf(entry).kind === "user" && !entry.readOnly
            ? t("editor:variables.details.editable")
            : t("editor:variables.details.readOnly")}
        </dd>
        {entry.deprecated && (
          <>
            <dt className="text-muted-foreground">{t("editor:variables.details.deprecated")}</dt>
            <dd>
              <Trans
                i18nKey="editor:variables.details.stillWorks"
                values={{ reference: formatReference(entry.deprecated.canonical) }}
                components={{ reference: <code /> }}
              />
            </dd>
          </>
        )}
        <dt className="text-muted-foreground">{t("editor:variables.details.changes")}</dt>
        <dd>
          {entry.dynamic
            ? t("editor:variables.details.dynamic")
            : t("editor:variables.details.static")}
        </dd>
        <dt className="text-muted-foreground">{t("editor:variables.details.availability")}</dt>
        <dd>{t(AVAILABILITY_LABEL[entry.availability])}</dd>
        {entry.sensitive && (
          <>
            <dt className="text-muted-foreground">{t("editor:variables.details.sensitive")}</dt>
            <dd>{t("editor:variables.details.neverShown")}</dd>
          </>
        )}
      </dl>
      <p className="text-muted-foreground" aria-label={t("editor:variables.details.context")}>
        {t("editor:variables.details.resolvedFor", {
          scope: describeContext(scope.context, scope.names),
        })}
      </p>
    </section>
  )
}
