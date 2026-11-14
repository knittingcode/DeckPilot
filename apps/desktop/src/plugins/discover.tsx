import { Search } from "lucide-react"
import { useState } from "react"

import { EmptyState } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Input } from "@/components/ui/input"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"

import { PluginIcon, PublisherLine } from "./components"
import type { MarketplaceActions } from "./marketplace"
import { categoryLabel, matchesQuery } from "./model"
import type { MarketplaceSnapshot, PluginCategory } from "./types"

export function DiscoverView({
  snapshot,
  actions,
}: {
  snapshot: MarketplaceSnapshot
  actions: MarketplaceActions
}) {
  const [query, setQuery] = useState("")
  const [category, setCategory] = useState<string>("all")
  // Deprecated plugins are hidden from browsing (architecture §13.5).
  const browsable = snapshot.catalog.filter((item) => !item.entry.deprecated)
  const categories = [...new Set(browsable.flatMap((item) => item.entry.categories ?? []))].sort(
    (a, b) => categoryLabel(a).localeCompare(categoryLabel(b)),
  )
  const shown = browsable
    .filter(
      (item) =>
        category === "all" || (item.entry.categories ?? []).includes(category as PluginCategory),
    )
    .filter((item) => matchesQuery(item, query))
    .sort((a, b) => a.entry.name.localeCompare(b.entry.name))
  const installed = new Map(snapshot.installed.map((p) => [p.id, p]))
  const updates = new Set(snapshot.updates.map((u) => u.id))

  if (browsable.length === 0) {
    return (
      <EmptyState title="No plugins to show">
        {snapshot.registry.state === "never-reached"
          ? "The registry has not been reached on this computer yet."
          : "The registry lists no plugins."}
      </EmptyState>
    )
  }

  return (
    <>
      <div className="flex flex-col gap-3">
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
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          value={category}
          onValueChange={(value) => {
            if (value) setCategory(value)
          }}
          aria-label="Category"
          className="flex-wrap"
        >
          <ToggleGroupItem value="all" className="px-2.5">
            All
          </ToggleGroupItem>
          {categories.map((value) => (
            <ToggleGroupItem key={value} value={value} className="px-2.5">
              {categoryLabel(value)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
      {shown.length === 0 ? (
        <EmptyState title="No plugins found">
          {query ? "No plugin matches your search." : "No plugin is in this category."}
        </EmptyState>
      ) : (
        <ul className="grid gap-3 sm:grid-cols-2" aria-label="Plugins">
          {shown.map((item) => {
            const local = installed.get(item.entry.id)
            return (
              <li key={item.entry.id} className="rounded-xl border bg-card">
                <button
                  type="button"
                  className="flex h-full w-full items-start gap-3 rounded-xl p-4 text-left outline-none hover:bg-accent/40 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                  aria-describedby={`plugin-${item.entry.id}-summary`}
                  onClick={() => {
                    actions.openDetails(item.entry.id)
                  }}
                >
                  <PluginIcon />
                  <span className="grid min-w-0 flex-1 gap-1">
                    <span className="font-medium">{item.entry.name}</span>
                    <PublisherLine publisher={item.publisher} namespace={item.entry.publisher} />
                    <span
                      id={`plugin-${item.entry.id}-summary`}
                      className="text-sm text-muted-foreground"
                    >
                      {item.entry.summary}
                    </span>
                    <span className="flex flex-wrap gap-1">
                      {(item.entry.categories ?? []).map((c) => (
                        <Badge key={c} variant="outline">
                          {categoryLabel(c)}
                        </Badge>
                      ))}
                      {local && <Badge variant="secondary">Installed {local.version}</Badge>}
                      {updates.has(item.entry.id) && <Badge>Update available</Badge>}
                    </span>
                  </span>
                </button>
              </li>
            )
          })}
        </ul>
      )}
    </>
  )
}
