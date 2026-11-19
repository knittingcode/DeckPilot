import { FileUp, RefreshCw, Search } from "lucide-react"
import { useState } from "react"

import { EmptyState } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"

import { useAppContext } from "@/lib/app-context"

import { Compatibility, PluginIcon, PublisherLine } from "./components"
import type { MarketplaceActions } from "./marketplace"
import {
  categoriesOf,
  categoryLabel,
  classifyPlugins,
  compareSemver,
  inCategory,
  matchesQuery,
  offeredVersion,
  type UpdateCandidate,
} from "./model"
import type { AvailablePlugin, MarketplaceSnapshot } from "./types"

type Classes = ReturnType<typeof classifyPlugins>

/**
 * What DeckPilot's functionality is made of: core functionality (the
 * built-in modules, always there and configured in Settings › Modules),
 * system plugins bundled with DeckPilot (origin `bundled`), the optional plugins
 * installed here, and the registry's plugins that are not installed.
 */
function Classification({ classes }: { classes: Classes }) {
  const rows: { title: string; text: string }[] = [
    {
      title: "Core functionality",
      text:
        classes.core.length > 0
          ? `Built into DeckPilot, not plugins: ${classes.core.map((m) => m.name).join(", ")}.`
          : "Built into DeckPilot, not plugins; see Settings › Modules.",
    },
    ...(classes.bundled.length > 0
      ? [
          {
            title: "Bundled system plugins",
            text: `${String(classes.bundled.length)} shipped with DeckPilot.`,
          },
        ]
      : []),
    {
      title: "Installed optional plugins",
      text:
        classes.installed.length === 0
          ? "None installed."
          : `${String(classes.installed.length)} installed by you; see Installed.`,
    },
    {
      title: "Available plugins",
      text:
        classes.available.length === 0
          ? "Nothing more in the registry."
          : `${String(classes.available.length)} in the registry, not installed.`,
    },
  ]
  return (
    <section aria-label="What DeckPilot includes" className="border-y py-2.5">
      <dl className="grid gap-x-6 gap-y-1 text-caption sm:grid-cols-[max-content_1fr]">
        {rows.map((row) => (
          <div key={row.title} className="contents">
            <dt className="text-muted-foreground">{row.title}</dt>
            <dd>{row.text}</dd>
          </div>
        ))}
      </dl>
    </section>
  )
}

/** What a card says about installing a registry plugin here. */
function installability(
  item: AvailablePlugin,
): { label: string; tone: "outline" | "destructive" } | null {
  if (offeredVersion(item)) return null
  const newest = [...item.versions].sort((a, b) => compareSemver(b.version, a.version))[0]
  if (newest?.withdrawn) return { label: "Withdrawn", tone: "destructive" }
  return { label: "Not compatible", tone: "outline" }
}

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
  const [category, setCategory] = useState<string | null>(null)
  const categories = categoriesOf(snapshot.available)
  const shown = snapshot.available
    .filter((item) => matchesQuery(item, query) && inCategory(item, category))
    .sort((a, b) => a.name.localeCompare(b.name))
  const updated = new Set(updates.map((u) => u.plugin.summary.id))
  const icons = new Map(snapshot.installed.map((p) => [p.summary.id, p.icon]))
  const { modules } = useAppContext()
  const classes = classifyPlugins(snapshot, modules)

  const toolbar = (
    <div className="flex flex-wrap items-center gap-2">
      <Button
        variant="outline"
        size="sm"
        disabled={actions.busy !== null || actions.blocked !== null}
        onClick={actions.installFile}
      >
        <FileUp /> Install from package file…
      </Button>
      <Button variant="ghost" size="sm" disabled={actions.busy !== null} onClick={actions.refresh}>
        <RefreshCw /> Refresh
      </Button>
      <span className="text-xs text-muted-foreground">
        Only a .dpplugin whose bytes the verified registry pins is accepted.
      </span>
    </div>
  )

  const overview = <Classification classes={classes} />

  if (snapshot.available.length === 0) {
    return (
      <>
        {overview}
        {toolbar}
        <EmptyState variant="plain" title="No plugins to show">
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
      {overview}
      {toolbar}
      <div className="relative">
        <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="search"
          placeholder="Search plugins"
          aria-label="Search plugins"
          className="h-8 pl-8"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
          }}
        />
      </div>
      {categories.length > 0 && (
        <div role="group" aria-label="Categories" className="flex flex-wrap gap-1.5">
          {[null, ...categories].map((c) => (
            <Button
              key={c ?? "all"}
              variant={category === c ? "secondary" : "ghost"}
              size="sm"
              className="h-7"
              aria-pressed={category === c}
              onClick={() => {
                setCategory(c)
              }}
            >
              {c === null ? "All categories" : categoryLabel(c)}
            </Button>
          ))}
        </div>
      )}
      {shown.length === 0 ? (
        <EmptyState variant="plain" title="No plugins found">
          {category === null
            ? "No plugin matches your search."
            : `No plugin in ${categoryLabel(category)} matches your search.`}
        </EmptyState>
      ) : (
        <ul className="grid divide-y border-y" aria-label="Plugins">
          {shown.map((item) => {
            const offered = offeredVersion(item)
            const blocked = installability(item)
            const versionShown =
              offered ?? [...item.versions].sort((a, b) => compareSemver(b.version, a.version))[0]
            return (
              <li key={item.id} className="py-1">
                <button
                  type="button"
                  className="flex w-full items-start gap-3 rounded-md px-2 py-2.5 text-left outline-none hover:bg-muted/60 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                  aria-describedby={`plugin-${item.id}-summary`}
                  onClick={() => {
                    actions.openDetails(item.id)
                  }}
                >
                  <PluginIcon muted={!offered && !item.installed} src={icons.get(item.id)} />
                  <span className="grid min-w-0 flex-1 gap-0.5">
                    <span className="text-sm font-medium">{item.name}</span>
                    <PublisherLine publisher={item.publisher} info={item.publisherInfo} />
                    <span
                      id={`plugin-${item.id}-summary`}
                      className="text-caption text-muted-foreground"
                    >
                      {item.summary}
                    </span>
                    {versionShown && <Compatibility version={versionShown} />}
                    <span className="mt-1 flex flex-wrap gap-1">
                      {item.latest && <Badge variant="outline">Latest {item.latest}</Badge>}
                      {item.installed ? (
                        <>
                          <Badge variant="secondary">Installed {item.installed}</Badge>
                          <Badge variant="outline">Optional plugin</Badge>
                        </>
                      ) : (
                        <Badge variant="outline">Available plugin</Badge>
                      )}
                      {updated.has(item.id) && <Badge>Update available</Badge>}
                      {!item.installed && blocked && (
                        <Badge variant={blocked.tone}>{blocked.label}</Badge>
                      )}
                      {item.deprecated && <Badge variant="outline">Deprecated</Badge>}
                      {(item.categories ?? []).map((c) => (
                        <Badge key={c} variant="outline" className="text-muted-foreground">
                          {categoryLabel(c)}
                        </Badge>
                      ))}
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
