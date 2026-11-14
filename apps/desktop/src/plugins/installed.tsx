import { ChevronDown, RefreshCw, RotateCcw, Trash2 } from "lucide-react"
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
import { Checkbox } from "@/components/ui/checkbox"
import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"

import { Notice, PermissionList, PluginIcon, PublisherLine } from "./components"
import type { MarketplaceActions } from "./marketplace"
import { grantsOf } from "./permissions"
import type { InstalledPlugin, InstalledProblem, MarketplaceSnapshot, PluginUpdate } from "./types"

export function InstalledView({
  snapshot,
  actions,
  onBrowse,
}: {
  snapshot: MarketplaceSnapshot
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
  const updates = new Map(snapshot.updates.map((u) => [u.id, u]))
  return (
    <ul className="grid gap-3" aria-label="Installed plugins">
      {[...snapshot.installed]
        .sort((a, b) => a.name.localeCompare(b.name))
        .map((item) => (
          <InstalledItem
            key={item.id}
            item={item}
            snapshot={snapshot}
            update={updates.get(item.id)}
            actions={actions}
          />
        ))}
    </ul>
  )
}

function InstalledItem({
  item,
  snapshot,
  update,
  actions,
}: {
  item: InstalledPlugin
  snapshot: MarketplaceSnapshot
  update: PluginUpdate | undefined
  actions: MarketplaceActions
}) {
  const [expanded, setExpanded] = useState(false)
  const id = `installed-${item.id}`
  const entry = snapshot.catalog.find((c) => c.entry.id === item.id)
  const revoked = item.problems.some((p) => p.kind === "revoked")
  const busy = actions.busy !== null
  const stored = new Map(item.grants.map((g) => [g.key, g]))
  const grants = grantsOf(item.permissions)
  const revokedCount = item.grants.filter((g) => !g.enabled).length

  return (
    <li
      className={cn("rounded-xl border bg-card p-4", !item.enabled && "bg-muted/30")}
      aria-labelledby={`${id}-name`}
    >
      <div className="flex items-start gap-3">
        <PluginIcon muted={!item.enabled} />
        <div className="grid min-w-0 flex-1 gap-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span id={`${id}-name`} className="font-medium">
              {item.name}
            </span>
            <span className="text-xs text-muted-foreground">v{item.version}</span>
            {item.source === "dev" && <Badge variant="outline">DEV</Badge>}
            {!item.enabled && <Badge variant="outline">Disabled</Badge>}
            {update && <Badge>Update {update.to} available</Badge>}
          </div>
          <PublisherLine publisher={entry?.publisher ?? null} namespace={item.publisher} />
          {revokedCount > 0 && (
            <p className="text-xs text-muted-foreground">
              {revokedCount} permission{revokedCount === 1 ? "" : "s"} revoked; features that need
              {revokedCount === 1 ? " it" : " them"} fail with “not permitted”.
            </p>
          )}
        </div>
        <Switch
          checked={item.enabled}
          disabled={busy || (revoked && !item.enabled)}
          aria-label={`${item.enabled ? "Disable" : "Enable"} ${item.name}`}
          aria-describedby={revoked ? `${id}-problem-revoked` : undefined}
          onCheckedChange={(enabled) => {
            actions.setEnabled(item, enabled)
          }}
        />
      </div>

      {item.problems.length > 0 && (
        <div className="mt-3 grid gap-2">
          {item.problems.map((problem) => (
            <ProblemNotice
              key={problem.kind}
              id={`${id}-problem-${problem.kind}`}
              item={item}
              problem={problem}
              update={update}
              actions={actions}
            />
          ))}
        </div>
      )}

      <div className="mt-3 flex flex-wrap gap-2 pl-13">
        {update && (
          <Button
            size="sm"
            disabled={busy || actions.blocked !== null}
            onClick={() => {
              actions.update(item, update)
            }}
          >
            <RefreshCw /> Update to {update.to}
          </Button>
        )}
        {entry && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              actions.openDetails(item.id)
            }}
          >
            Details
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          aria-expanded={expanded}
          aria-controls={`${id}-permissions`}
          onClick={() => {
            setExpanded(!expanded)
          }}
        >
          <ChevronDown className={cn("transition-transform", expanded && "rotate-180")} />
          Permissions
        </Button>
        <UninstallButton item={item} disabled={busy} actions={actions} />
      </div>

      {expanded && (
        <div id={`${id}-permissions`} className="mt-3 border-t pt-3">
          <PermissionList
            label={`Permissions of ${item.name}`}
            grants={grants}
            renderControl={(grant) => {
              const g = stored.get(grant.key)
              if (!g) return <Badge variant="outline">Not granted</Badge>
              return g.enabled ? (
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
              ) : (
                <div className="flex shrink-0 items-center gap-2">
                  <Badge variant="outline">Revoked</Badge>
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={busy}
                    aria-label={`Allow again: ${grant.title}`}
                    onClick={() => {
                      actions.setGrant(item, grant.key, true)
                    }}
                  >
                    Allow
                  </Button>
                </div>
              )
            }}
          />
        </div>
      )}
    </li>
  )
}

function ProblemNotice({
  id,
  item,
  problem,
  update,
  actions,
}: {
  id: string
  item: InstalledPlugin
  problem: InstalledProblem
  update: PluginUpdate | undefined
  actions: MarketplaceActions
}) {
  const rollback = item.previousVersion && (
    <div>
      <Button
        variant="outline"
        size="sm"
        disabled={actions.busy !== null}
        onClick={() => {
          actions.rollback(item)
        }}
      >
        <RotateCcw /> Roll back to {item.previousVersion}
      </Button>
    </div>
  )
  switch (problem.kind) {
    case "broken":
      return (
        <div id={id}>
          <Notice tone="destructive" title={`${item.name} failed to load`}>
            <p>
              {problem.message} <span className="font-mono text-xs">({problem.code})</span>
            </p>
            <p>{problem.hint}</p>
            {rollback}
          </Notice>
        </div>
      )
    case "revoked":
      return (
        <div id={id}>
          <Notice tone="destructive" title="Disabled: revoked by the registry">
            <p>{problem.reason}</p>
            <p>
              It cannot run or be enabled. Its data is kept until you uninstall it.
              {update ? ` Version ${update.to} is not revoked.` : ""}
              {problem.advisory && (
                <>
                  {" "}
                  Advisory: <span className="select-text">{problem.advisory}</span>
                </>
              )}
            </p>
            {rollback}
          </Notice>
        </div>
      )
    case "yanked":
      return (
        <div id={id}>
          <Notice title={`Version ${item.version} was withdrawn`}>
            <p>{problem.reason}</p>
            <p>
              It keeps running.{" "}
              {update
                ? `Updating to ${update.to} is recommended.`
                : "No replacement is offered yet."}
            </p>
          </Notice>
        </div>
      )
    case "deprecated":
      return (
        <div id={id}>
          <Notice title="Deprecated">
            <p>
              {problem.reason}
              {problem.replacement && ` Replacement: ${problem.replacement}.`}
            </p>
          </Notice>
        </div>
      )
    case "incompatible":
      return (
        <div id={id}>
          <Notice title="Not compatible with this DeckPilot">
            <p>{problem.reason}</p>
            <p>Update the plugin, or DeckPilot, to use it again.</p>
          </Notice>
        </div>
      )
  }
}

function UninstallButton({
  item,
  disabled,
  actions,
}: {
  item: InstalledPlugin
  disabled: boolean
  actions: MarketplaceActions
}) {
  const [deleteCredentials, setDeleteCredentials] = useState(false)
  const hasCredentials = item.permissions.some((p) => p.kind === "credentials")
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button variant="ghost" size="sm" disabled={disabled}>
          <Trash2 /> Uninstall
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Uninstall {item.name}?</AlertDialogTitle>
          <AlertDialogDescription>
            Removes the plugin, its permissions, its storage and the values of its variables.
            Actions that use it are kept and shown as unavailable.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {hasCredentials && (
          <div className="flex items-center gap-2 text-sm">
            <Checkbox
              id={`uninstall-${item.id}-credentials`}
              checked={deleteCredentials}
              onCheckedChange={(value) => {
                setDeleteCredentials(value === true)
              }}
            />
            <label htmlFor={`uninstall-${item.id}-credentials`}>
              Also delete the secrets stored for it
            </label>
          </div>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-white hover:bg-destructive/90"
            onClick={() => {
              actions.uninstall(item, deleteCredentials)
            }}
          >
            Uninstall
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
