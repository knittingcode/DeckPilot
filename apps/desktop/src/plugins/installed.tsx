import { ChevronDown, Play, RefreshCw, RotateCcw, Trash2 } from "lucide-react"
import { useState } from "react"

import { EmptyState } from "@/components/page"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"

import { pluginErrorText } from "./api"
import { Notice, PermissionList, PluginIcon, PublisherLine } from "./components"
import type { MarketplaceActions } from "./marketplace"
import { STATE_LABELS, type UpdateCandidate } from "./model"
import { grantOfInfo } from "./permissions"
import type {
  ActionParamValue,
  MarketplaceSnapshot,
  PluginActionInfo,
  PluginActionOutcome,
  PluginDetails,
} from "./types"

export function InstalledView({
  snapshot,
  updates,
  actions,
  onBrowse,
}: {
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  actions: MarketplaceActions
  onBrowse: () => void
}) {
  if (snapshot.installed.length === 0) {
    return (
      <EmptyState title="No plugins installed">
        <Button variant="outline" size="sm" className="mt-2" onClick={onBrowse}>
          Browse plugins
        </Button>
      </EmptyState>
    )
  }
  const byId = new Map(updates.map((u) => [u.plugin.summary.id, u]))
  return (
    <ul className="grid gap-3" aria-label="Installed plugins">
      {[...snapshot.installed]
        .sort((a, b) => a.summary.name.localeCompare(b.summary.name))
        .map((item) => (
          <InstalledItem
            key={item.summary.id}
            item={item}
            listed={snapshot.available.some((p) => p.id === item.summary.id)}
            update={byId.get(item.summary.id)}
            actions={actions}
          />
        ))}
    </ul>
  )
}

function InstalledItem({
  item,
  listed,
  update,
  actions,
}: {
  item: PluginDetails
  listed: boolean
  update: UpdateCandidate | undefined
  actions: MarketplaceActions
}) {
  const [expanded, setExpanded] = useState(false)
  const { summary } = item
  const id = `installed-${summary.id}`
  const enabled = summary.state === "enabled"
  const busy = actions.busy !== null
  const notGranted = item.permissions.filter((p) => p.status !== "granted").length
  const blockedState = summary.state === "broken" || summary.state === "quarantined"
  const updateVersion = update
    ? update.kind === "staged"
      ? update.pending.version
      : update.version
    : undefined

  return (
    <li
      className={cn("rounded-xl border bg-card p-4", !enabled && "bg-muted/30")}
      aria-labelledby={`${id}-name`}
    >
      <div className="flex items-start gap-3">
        <PluginIcon muted={!enabled} />
        <div className="grid min-w-0 flex-1 gap-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span id={`${id}-name`} className="font-medium">
              {summary.name}
            </span>
            <span className="text-xs text-muted-foreground">v{summary.version}</span>
            {summary.origin.type === "dev" && <Badge variant="outline">DEV</Badge>}
            {summary.origin.type === "sideload" && <Badge variant="outline">Local package</Badge>}
            <Badge variant={blockedState ? "destructive" : "outline"}>
              {STATE_LABELS[summary.state]}
            </Badge>
            {updateVersion && <Badge>Update {updateVersion} available</Badge>}
          </div>
          <PublisherLine publisher={summary.publisher} />
          {notGranted > 0 && (
            <p className="text-xs text-muted-foreground">
              {notGranted} permission{notGranted === 1 ? "" : "s"} not granted; calls that need
              {notGranted === 1 ? " it" : " them"} fail with “not permitted”.
            </p>
          )}
        </div>
        <Switch
          checked={enabled}
          disabled={busy || blockedState || (!enabled && !actions.canRun)}
          aria-label={`${enabled ? "Disable" : "Enable"} ${summary.name}`}
          onCheckedChange={(value) => {
            actions.setEnabled(item, value)
          }}
        />
      </div>

      {(blockedState || summary.stateReason) && (
        <div className="mt-3">
          <Notice
            tone={blockedState ? "destructive" : "default"}
            title={
              summary.state === "broken"
                ? `${summary.name} is broken`
                : summary.state === "quarantined"
                  ? "Disabled: revoked by the registry"
                  : `${summary.name}: ${STATE_LABELS[summary.state].toLowerCase()}`
            }
          >
            {summary.stateReason && <p>{summary.stateReason}</p>}
            {blockedState && <p>It cannot run or be enabled. Roll back, update or uninstall it.</p>}
          </Notice>
        </div>
      )}

      {update?.kind === "staged" && (
        <div className="mt-3">
          <Notice title={`Version ${update.pending.version} waits for your consent`}>
            <p>
              It asks for{" "}
              {update.pending.changes.added.length + update.pending.changes.broadened.length} more
              permission
              {update.pending.changes.added.length + update.pending.changes.broadened.length === 1
                ? ""
                : "s"}
              . The current version keeps running until you decide.
            </p>
          </Notice>
        </div>
      )}

      <div className="mt-3 flex flex-wrap gap-2 pl-13">
        {update && (
          <Button
            size="sm"
            disabled={busy || actions.blocked !== null}
            onClick={() => {
              actions.update(update)
            }}
          >
            <RefreshCw />
            {update.kind === "staged"
              ? `Review update to ${updateVersion ?? ""}`
              : `Update to ${updateVersion ?? ""}`}
          </Button>
        )}
        {summary.previousVersion && (
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => {
              actions.rollback(item)
            }}
          >
            <RotateCcw /> Roll back to {summary.previousVersion}
          </Button>
        )}
        {listed && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              actions.openDetails(summary.id)
            }}
          >
            Details
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          aria-expanded={expanded}
          aria-controls={`${id}-more`}
          onClick={() => {
            setExpanded(!expanded)
          }}
        >
          <ChevronDown className={cn("transition-transform", expanded && "rotate-180")} />
          Permissions and actions
        </Button>
        <UninstallButton item={item} disabled={busy} actions={actions} />
      </div>

      {expanded && (
        <div id={`${id}-more`} className="mt-3 grid gap-4 border-t pt-3">
          <PermissionList
            label={`Permissions of ${summary.name}`}
            grants={item.permissions.map(grantOfInfo)}
            renderControl={(grant) => {
              const info = item.permissions.find((p) => p.key === grant.key)
              if (info?.status === "granted") {
                return (
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    aria-label={`Revoke: ${grant.title}`}
                    onClick={() => {
                      actions.setGrant(item, grant.key, false)
                    }}
                  >
                    Revoke
                  </Button>
                )
              }
              return (
                <div className="flex shrink-0 items-center gap-2">
                  <Badge variant="outline">
                    {info?.status === "revoked" ? "Revoked" : "Not granted"}
                  </Badge>
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={busy}
                    aria-label={`Grant: ${grant.title}`}
                    onClick={() => {
                      actions.setGrant(item, grant.key, true)
                    }}
                  >
                    Grant
                  </Button>
                </div>
              )
            }}
          />
          <ActionList item={item} actions={actions} />
        </div>
      )}
    </li>
  )
}

interface ParameterSpec {
  key: string
  label: string
  type: string
  options?: string[]
  defaultValue?: ActionParamValue
}

/** The manifest's parameters of `action` (`contributes.actions[].parameters`). */
function actionParameters(manifest: unknown, action: string): ParameterSpec[] {
  const record = (v: unknown) =>
    typeof v === "object" && v !== null ? (v as Record<string, unknown>) : undefined
  const actions = record(record(manifest)?.contributes)?.actions
  if (!Array.isArray(actions)) return []
  const entry = actions.map(record).find((a) => a?.id === action)
  const parameters = entry?.parameters
  if (!Array.isArray(parameters)) return []
  return parameters.flatMap((p): ParameterSpec[] => {
    const r = record(p)
    if (!r || typeof r.key !== "string") return []
    const kind = record(r.kind)
    const spec: ParameterSpec = {
      key: r.key,
      label: typeof r.label === "string" ? r.label : r.key,
      type: typeof kind?.type === "string" ? kind.type : "text",
    }
    if (Array.isArray(kind?.options)) {
      spec.options = kind.options.filter((o): o is string => typeof o === "string")
    }
    const value = record(r.default)?.value
    if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
      spec.defaultValue = value
    }
    return [spec]
  })
}

function ActionList({ item, actions }: { item: PluginDetails; actions: MarketplaceActions }) {
  if (item.summary.actions.length === 0) return null
  return (
    <section className="grid gap-2" aria-label={`Actions of ${item.summary.name}`}>
      <h4 className="text-xs font-medium text-muted-foreground uppercase">Actions</h4>
      <ul className="grid gap-2">
        {item.summary.actions.map((action) => (
          <ActionRow key={action.action} item={item} action={action} actions={actions} />
        ))}
      </ul>
    </section>
  )
}

function ActionRow({
  item,
  action,
  actions,
}: {
  item: PluginDetails
  action: PluginActionInfo
  actions: MarketplaceActions
}) {
  const parameters = actionParameters(item.manifest, action.action)
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(
      parameters.map((p) => [p.key, p.defaultValue === undefined ? "" : String(p.defaultValue)]),
    ),
  )
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<
    { outcome: PluginActionOutcome } | { error: { message: string; hint?: string } } | null
  >(null)

  const run = () => {
    const params: Record<string, ActionParamValue> = {}
    for (const p of parameters) {
      const raw = values[p.key] ?? ""
      if (raw === "") continue
      params[p.key] =
        p.type === "number" ? Number(raw) : p.type === "boolean" ? raw === "true" : raw
    }
    setRunning(true)
    setResult(null)
    actions
      .invoke(item, action.action, params)
      .then(
        (outcome) => {
          setResult({ outcome })
        },
        (error: unknown) => {
          setResult({ error: pluginErrorText(error) })
        },
      )
      .finally(() => {
        setRunning(false)
      })
  }

  const id = `action-${item.summary.id}-${action.action}`
  return (
    <li className="grid gap-2 rounded-lg border p-3" aria-labelledby={`${id}-name`}>
      <div className="flex flex-wrap items-center gap-2">
        <span id={`${id}-name`} className="text-sm font-medium">
          {action.name}
        </span>
        <span className="font-mono text-xs text-muted-foreground">{action.actionType}</span>
        <Button
          size="sm"
          variant="outline"
          className="ml-auto"
          disabled={running || !action.available}
          aria-label={`Run ${action.name}`}
          onClick={run}
        >
          <Play /> Run
        </Button>
      </div>
      {action.description && <p className="text-xs text-muted-foreground">{action.description}</p>}
      {!action.available && action.unavailableReason && (
        <p className="text-xs text-muted-foreground">{action.unavailableReason}</p>
      )}
      {action.available && parameters.length > 0 && (
        <div className="grid gap-2 sm:grid-cols-2">
          {parameters.map((p) => (
            <label key={p.key} className="grid gap-1 text-xs">
              <span>{p.label}</span>
              <Input
                value={values[p.key] ?? ""}
                type={p.type === "number" ? "number" : "text"}
                placeholder={p.options ? p.options.join(" / ") : undefined}
                onChange={(e) => {
                  setValues({ ...values, [p.key]: e.target.value })
                }}
              />
            </label>
          ))}
        </div>
      )}
      {result && (
        <div role="status" aria-label={`Result of ${action.name}`} className="text-xs">
          {"error" in result ? (
            <p className="text-destructive">
              {result.error.message}
              {result.error.hint ? ` ${result.error.hint}` : ""}
            </p>
          ) : result.outcome.status === "ok" ? (
            <div className="grid gap-1">
              <p>Succeeded{result.outcome.message ? `: ${result.outcome.message}` : ""}</p>
              {result.outcome.data && Object.keys(result.outcome.data).length > 0 && (
                <pre className="overflow-x-auto rounded bg-muted p-2 select-text">
                  {JSON.stringify(result.outcome.data, null, 2)}
                </pre>
              )}
            </div>
          ) : result.outcome.status === "skipped" ? (
            <p>Skipped{result.outcome.message ? `: ${result.outcome.message}` : ""}</p>
          ) : (
            <p className="text-destructive">
              Failed ({result.outcome.code})
              {result.outcome.message ? `: ${result.outcome.message}` : ""}
            </p>
          )}
        </div>
      )}
    </li>
  )
}

function UninstallButton({
  item,
  disabled,
  actions,
}: {
  item: PluginDetails
  disabled: boolean
  actions: MarketplaceActions
}) {
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button variant="ghost" size="sm" disabled={disabled}>
          <Trash2 /> Uninstall
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Uninstall {item.summary.name}?</AlertDialogTitle>
          <AlertDialogDescription>
            Stops the plugin and removes its files, its permissions and its records, including the
            version kept for rollback.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-white hover:bg-destructive/90"
            onClick={() => {
              actions.uninstall(item)
            }}
          >
            Uninstall
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
