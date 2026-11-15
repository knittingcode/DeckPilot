import { FileUp, Search } from "lucide-react"
import { useState } from "react"

import { EmptyState } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"

import { PluginIcon, PublisherLine } from "./components"
import type { MarketplaceActions } from "./marketplace"
import { matchesQuery, type UpdateCandidate } from "./model"
import type { MarketplaceSnapshot } from "./types"

export function DiscoverView({
  snapshot,
  updates,
  actions,
}: {
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  actions: MarketplaceActions
}) {
  const [query, setQuery] = useState("")
  const shown = snapshot.available
    .filter((item) => matchesQuery(item, query))
    .sort((a, b) => a.name.localeCompare(b.name))
  const updated = new Set(updates.map((u) => u.plugin.summary.id))

  const fromFile = (
    <div className="flex flex-wrap items-center gap-2">
      <Button
        variant="outline"
        size="sm"
        disabled={actions.busy !== null || actions.blocked !== null}
        onClick={actions.installFile}
      >
        <FileUp /> Install from package file…
      </Button>
      <span className="text-xs text-muted-foreground">
        Only a .dpplugin whose bytes the verified registry pins is accepted.
      </span>
    </div>
  )

  if (snapshot.available.length === 0) {
    return (
      <>
        {fromFile}
        <EmptyState title="No plugins to show">
          {snapshot.registryError
            ? "The registry could not be read."
            : !snapshot.registry
              ? "No registry is configured in the agent."
              : snapshot.registry.freshness === "neverVerified"
                ? "The registry has not been verified on this computer yet."
                : "The registry lists no plugins."}
        </EmptyState>
      </>
    )
  }

  return (
    <>
      {fromFile}
      <div className="relative">
        <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="search"
          placeholder="Search plugins"
          aria-label="Search plugins"
          className="pl-8"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
          }}
        />
      </div>
      {shown.length === 0 ? (
        <EmptyState title="No plugins found">No plugin matches your search.</EmptyState>
      ) : (
        <ul className="grid gap-3 sm:grid-cols-2" aria-label="Plugins">
          {shown.map((item) => (
            <li key={item.id} className="rounded-xl border bg-card">
              <button
                type="button"
                className="flex h-full w-full items-start gap-3 rounded-xl p-4 text-left outline-none hover:bg-accent/40 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                aria-describedby={`plugin-${item.id}-summary`}
                onClick={() => {
                  actions.openDetails(item.id)
                }}
              >
                <PluginIcon />
                <span className="grid min-w-0 flex-1 gap-1">
                  <span className="font-medium">{item.name}</span>
                  <PublisherLine publisher={item.publisher} />
                  <span id={`plugin-${item.id}-summary`} className="text-sm text-muted-foreground">
                    {item.summary}
                  </span>
                  <span className="flex flex-wrap gap-1">
                    {item.latest && <Badge variant="outline">{item.latest}</Badge>}
                    {item.installed && (
                      <Badge variant="secondary">Installed {item.installed}</Badge>
                    )}
                    {updated.has(item.id) && <Badge>Update available</Badge>}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </>
  )
}
