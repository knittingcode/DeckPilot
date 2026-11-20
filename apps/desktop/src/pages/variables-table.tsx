import { ClipboardCopy, Lock, Pencil, Search } from "lucide-react"
import { useContext, useMemo, useState } from "react"
import { toast } from "sonner"

import { PreviewValue } from "@/components/variable-picker/variable-results"
import { useVariableScope } from "@/components/variable-picker/scope"
import {
  useReferencePreviews,
  useVariableCatalog,
} from "@/components/variable-picker/use-variables"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useTranslation } from "@/i18n"
import { AppContext } from "@/lib/app-context"
import { SYSTEM_VARIABLES, VALUE_TYPES, typeLabel } from "@/lib/shared-variables"
import type { CatalogEntry, SharedVariable } from "@/lib/types"
import {
  CATALOG_SCOPE_LABEL,
  type CatalogScope,
  NO_FILTER,
  type OwnerFilter,
  type RowFilter,
  type VariableRow,
  catalogRows,
  filterRows,
  ownerKey,
  ownerLabel,
} from "@/lib/variable-catalog"
import { AVAILABILITY_LABEL, formatReference } from "@/lib/variables"

/** Most references one preview request may ask about (agent limit). */
const MAX_PREVIEWS = 200

/**
 * Every variable in one table: core, module and user variables, with
 * search and owner, type and scope filters. Values come from the agent
 * and are never shown for sensitive variables. Only user variables can be
 * edited; `onEdit` opens the editor for one.
 */
export function VariablesTable({
  shared,
  selected,
  onEdit,
  namespace,
}: {
  shared: SharedVariable[]
  selected: string | null
  onEdit: (key: string) => void
  /** Only the variables under this namespace, e.g. a plugin's `plugins.acme.weather`. */
  namespace?: string
}) {
  const { t } = useTranslation()
  const app = useContext(AppContext)
  const scope = useVariableScope()
  const [filter, setFilter] = useState<RowFilter>(NO_FILTER)
  const live = Boolean(scope?.live)
  const catalogScope = scope ?? {
    context: { profile: "" },
    names: {},
    live: false,
    stepOutputs: [],
  }
  const catalog = useVariableCatalog(catalogScope, "", undefined, live)
  // Without the agent: the known core ids, without values.
  const entries: CatalogEntry[] = live ? catalog.entries : SYSTEM_VARIABLES
  const rows = useMemo(() => {
    const all = catalogRows(entries, shared)
    return namespace ? all.filter((row) => row.reference.startsWith(`${namespace}.`)) : all
  }, [entries, shared, namespace])
  const shown = filterRows(rows, filter)
  const previews = useReferencePreviews(
    catalogScope,
    rows
      .filter((row) => !row.sensitive)
      .slice(0, MAX_PREVIEWS)
      .map((row) => ({ reference: row.reference })),
    live,
  )
  const moduleNames = useMemo(
    () => Object.fromEntries((app?.modules ?? []).map((m) => [m.id, m.name])),
    [app?.modules],
  )
  const owners = useMemo(() => {
    const seen = new Map<OwnerFilter, string>()
    for (const row of rows) seen.set(ownerKey(row.owner), ownerLabel(row.owner, moduleNames))
    return [...seen.entries()].sort((a, b) => a[1].localeCompare(b[1]))
  }, [rows, moduleNames])
  const scopes = useMemo(() => [...new Set(rows.flatMap((row) => row.scopes))].sort(), [rows])

  return (
    <div className="grid gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative min-w-48 flex-1">
          <Search className="absolute top-2.5 left-2 size-4 text-muted-foreground" />
          <Input
            className="pl-8"
            placeholder={t("workflows:variablesTable.search")}
            aria-label={t("workflows:variablesTable.search")}
            value={filter.query}
            onChange={(e) => {
              setFilter({ ...filter, query: e.target.value })
            }}
          />
        </div>
        <FilterSelect
          label={t("workflows:variablesTable.owner")}
          filterLabel={t("workflows:variablesTable.filter.owner")}
          anyLabel={t("workflows:variablesTable.any.owner")}
          value={filter.owner}
          options={owners}
          onChange={(owner) => {
            setFilter({ ...filter, owner: owner as OwnerFilter })
          }}
        />
        <FilterSelect
          label={t("workflows:variablesTable.type")}
          filterLabel={t("workflows:variablesTable.filter.type")}
          anyLabel={t("workflows:variablesTable.any.type")}
          value={filter.type}
          options={VALUE_TYPES.map((v) => [v.type, v.label])}
          onChange={(type) => {
            setFilter({ ...filter, type: type as RowFilter["type"] })
          }}
        />
        <FilterSelect
          label={t("workflows:variablesTable.scope")}
          filterLabel={t("workflows:variablesTable.filter.scope")}
          anyLabel={t("workflows:variablesTable.any.scope")}
          value={filter.scope}
          options={scopes.map((s) => [s, t(CATALOG_SCOPE_LABEL[s])])}
          onChange={(value) => {
            setFilter({ ...filter, scope: value as CatalogScope | "all" })
          }}
        />
      </div>
      {catalog.unavailable && (
        <p className="text-xs text-muted-foreground" role="status">
          {t("workflows:variablesTable.valuesUnavailable", { reason: catalog.unavailable })}
        </p>
      )}
      {!live && (
        <p className="text-xs text-muted-foreground" role="status">
          {t("workflows:variablesTable.notLive")}
        </p>
      )}
      {shown.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("workflows:variablesTable.noMatch")}</p>
      ) : (
        <Table aria-label={t("workflows:variablesTable.label")}>
          <TableHeader>
            <TableRow>
              <TableHead>{t("workflows:variablesTable.variable")}</TableHead>
              <TableHead>{t("workflows:variablesTable.owner")}</TableHead>
              <TableHead>{t("workflows:variablesTable.type")}</TableHead>
              <TableHead>{t("workflows:variablesTable.scope")}</TableHead>
              <TableHead>{t("workflows:variablesTable.currentValue")}</TableHead>
              <TableHead>{t("workflows:variablesTable.access")}</TableHead>
              <TableHead>
                <span className="sr-only">{t("workflows:variablesTable.actions")}</span>
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {shown.map((row) => (
              <VariableTableRow
                key={row.reference}
                row={row}
                selected={selected !== null && row.sharedKey === selected}
                ownerName={ownerLabel(row.owner, moduleNames)}
                value={
                  row.sensitive ? (
                    // Never asked for: only ever shown as redacted.
                    <span className="text-muted-foreground italic">
                      {t("workflows:variablesTable.redacted")}
                    </span>
                  ) : (
                    <PreviewValue
                      status={previews.status(row.reference)}
                      sensitive={row.sensitive}
                      live={live}
                      loading={previews.loading}
                      error={previews.error}
                    />
                  )
                }
                onEdit={onEdit}
              />
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  )
}

function FilterSelect({
  filterLabel,
  anyLabel,
  value,
  options,
  onChange,
}: {
  /** The column, as the table heads it. */
  label: string
  filterLabel: string
  anyLabel: string
  value: string
  options: [string, string][]
  onChange: (value: string) => void
}) {
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger className="w-40" aria-label={filterLabel}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="all">{anyLabel}</SelectItem>
        {options.map(([key, text]) => (
          <SelectItem key={key} value={key}>
            {text}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

function VariableTableRow({
  row,
  selected,
  ownerName,
  value,
  onEdit,
}: {
  row: VariableRow
  selected: boolean
  ownerName: string
  value: React.ReactNode
  onEdit: (key: string) => void
}) {
  const { t } = useTranslation()
  const editable = row.owner.kind === "user" && row.sharedKey !== undefined
  const copy = () => {
    navigator.clipboard.writeText(formatReference(row.reference)).then(
      () => toast.success(t("workflows:variablesTable.toast.copied")),
      () => toast.error(t("workflows:variablesTable.toast.copyFailed")),
    )
  }
  return (
    <TableRow aria-label={row.reference} data-state={selected ? "selected" : undefined}>
      <TableCell className="max-w-72 align-top whitespace-normal">
        <span className="block font-medium">{row.name}</span>
        <code className="block text-xs break-all text-muted-foreground">
          {formatReference(row.reference)}
        </code>
        {row.description && (
          <span className="block text-xs text-muted-foreground">{row.description}</span>
        )}
        {row.aliases.length > 0 && (
          <span className="block text-xs text-muted-foreground">
            {t("workflows:variablesTable.aliases", {
              aliases: row.aliases.map((a) => formatReference(a)).join(", "),
            })}
          </span>
        )}
      </TableCell>
      <TableCell className="align-top">{ownerName}</TableCell>
      <TableCell className="align-top">{typeLabel(row.valueType)}</TableCell>
      <TableCell className="align-top whitespace-normal">
        {row.scopes.map((s) => t(CATALOG_SCOPE_LABEL[s])).join(", ") || "—"}
      </TableCell>
      <TableCell className="max-w-56 align-top text-xs whitespace-normal">
        {value}
        {row.availability !== "available" && (
          <span className="block text-muted-foreground">
            {t(AVAILABILITY_LABEL[row.availability])}
          </span>
        )}
      </TableCell>
      <TableCell className="align-top">
        <span className="flex flex-wrap gap-1">
          {editable ? (
            <Badge variant="secondary">{t("workflows:variablesTable.editable")}</Badge>
          ) : (
            <Badge variant="outline">{t("workflows:variablesTable.readOnly")}</Badge>
          )}
          {row.sensitive && (
            <Badge variant="outline">
              <Lock /> {t("workflows:variablesTable.sensitive")}
            </Badge>
          )}
          {row.usages !== undefined && row.usages > 0 && (
            <Badge
              variant="secondary"
              title={t("workflows:variablesTable.usedTimes", { count: row.usages })}
            >
              {t("workflows:variablesTable.used", { count: row.usages })}
            </Badge>
          )}
        </span>
      </TableCell>
      <TableCell className="align-top">
        <span className="flex gap-1">
          <Button
            type="button"
            variant="ghost"
            size="icon"
            aria-label={t("workflows:variablesTable.copy", { reference: row.reference })}
            onClick={copy}
          >
            <ClipboardCopy />
          </Button>
          {editable && row.sharedKey && (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label={t("workflows:variablesTable.edit", { reference: row.reference })}
              onClick={() => {
                if (row.sharedKey) onEdit(row.sharedKey)
              }}
            >
              <Pencil />
            </Button>
          )}
        </span>
      </TableCell>
    </TableRow>
  )
}
