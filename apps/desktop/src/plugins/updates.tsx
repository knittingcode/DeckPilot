import { RefreshCw } from "lucide-react"

import { EmptyState } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"

import type { MarketplaceActions } from "./marketplace"
import { compareGrants } from "./permissions"
import type { MarketplaceSnapshot } from "./types"

/** Newer versions of installed plugins; new permissions are always asked for. */
export function UpdatesView({
  snapshot,
  actions,
}: {
  snapshot: MarketplaceSnapshot
  actions: MarketplaceActions
}) {
  const checked = snapshot.registry.checkedAt
  return (
    <>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm text-muted-foreground">
          Last checked: {checked ? new Date(checked).toLocaleString() : "never"}. Updates that ask
          for new permissions are never applied without your consent.
        </p>
        <Button
          variant="outline"
          size="sm"
          disabled={actions.busy !== null}
          onClick={actions.refresh}
        >
          <RefreshCw /> Check for updates
        </Button>
      </div>
      {actions.blocked && <p className="text-sm text-muted-foreground">{actions.blocked}</p>}
      {snapshot.updates.length === 0 ? (
        <EmptyState title="All plugins are up to date" />
      ) : (
        <ul className="grid gap-3" aria-label="Available updates">
          {snapshot.updates.map((update) => {
            const item = snapshot.installed.find((p) => p.id === update.id)
            if (!item) return null
            const added = compareGrants(item.grants, update.permissions).added
            return (
              <li
                key={update.id}
                className="flex flex-wrap items-center gap-3 rounded-xl border bg-card p-4"
                aria-label={`${item.name} ${update.from} to ${update.to}`}
              >
                <div className="grid min-w-0 flex-1 gap-1">
                  <span className="font-medium">{item.name}</span>
                  <span className="text-xs text-muted-foreground">
                    {update.from} → {update.to} · published{" "}
                    {new Date(update.publishedAt).toLocaleDateString()}
                  </span>
                </div>
                {added.length > 0 ? (
                  <Badge variant="outline">
                    {added.length} new permission{added.length === 1 ? "" : "s"}
                  </Badge>
                ) : (
                  <Badge variant="secondary">No new permissions</Badge>
                )}
                <Button
                  size="sm"
                  disabled={actions.busy !== null || actions.blocked !== null}
                  onClick={() => {
                    actions.update(item, update)
                  }}
                >
                  {added.length > 0 ? "Review and update" : "Update"}
                </Button>
              </li>
            )
          })}
        </ul>
      )}
    </>
  )
}
