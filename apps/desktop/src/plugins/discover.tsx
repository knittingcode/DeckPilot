import { FileUp, RefreshCw, Search } from "lucide-react"
import { useState } from "react"

import { EmptyState } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"

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
  const { t } = useTranslation()
  const rows: { title: string; text: string }[] = [
    {
      title: t("marketplace:classes.core.title"),
      text:
        classes.core.length > 0
          ? t("marketplace:classes.core.list", {
              modules: classes.core.map((m) => m.name).join(", "),
            })
          : t("marketplace:classes.core.none"),
    },
    ...(classes.bundled.length > 0
      ? [
          {
            title: t("marketplace:classes.bundled.title"),
            text: t("marketplace:classes.bundled.text", { count: classes.bundled.length }),
          },
        ]
      : []),
    {
      title: t("marketplace:classes.installed.title"),
      text:
        classes.installed.length === 0
          ? t("marketplace:classes.installed.none")
          : t("marketplace:classes.installed.text", { count: classes.installed.length }),
    },
    {
      title: t("marketplace:classes.available.title"),
      text:
        classes.available.length === 0
          ? t("marketplace:classes.available.none")
          : t("marketplace:classes.available.text", { count: classes.available.length }),
    },
  ]
  return (
    <section aria-label={t("marketplace:classes.label")} className="border-y py-2.5">
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
): { label: "withdrawn" | "incompatible"; tone: "outline" | "destructive" } | null {
  if (offeredVersion(item)) return null
  const newest = [...item.versions].sort((a, b) => compareSemver(b.version, a.version))[0]
  if (newest?.withdrawn) return { label: "withdrawn", tone: "destructive" }
  return { label: "incompatible", tone: "outline" }
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
  const { t } = useTranslation()
  const format = useFormat()
  const [query, setQuery] = useState("")
  const [category, setCategory] = useState<string | null>(null)
  const categories = categoriesOf(snapshot.available)
  const shown = snapshot.available
    .filter((item) => matchesQuery(item, query) && inCategory(item, category))
    .sort((a, b) => format.compare(a.name, b.name))
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
        <FileUp /> {t("marketplace:discover.installFile")}
      </Button>
      <Button variant="ghost" size="sm" disabled={actions.busy !== null} onClick={actions.refresh}>
        <RefreshCw /> {t("marketplace:refresh")}
      </Button>
      <span className="text-xs text-muted-foreground">
        {t("marketplace:discover.installFileHint")}
      </span>
    </div>
  )

  const overview = <Classification classes={classes} />

  if (snapshot.available.length === 0) {
    return (
      <>
        {overview}
        {toolbar}
        <EmptyState variant="plain" title={t("marketplace:discover.empty.title")}>
          {snapshot.registryError
            ? t("marketplace:discover.empty.registryError")
            : !snapshot.registry
              ? t("marketplace:discover.empty.noRegistry")
              : snapshot.registry.freshness === "neverVerified"
                ? t("marketplace:discover.empty.neverVerified")
                : t("marketplace:discover.empty.none")}
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
          placeholder={t("marketplace:discover.search")}
          aria-label={t("marketplace:discover.search")}
          className="h-8 pl-8"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
          }}
        />
      </div>
      {categories.length > 0 && (
        <div
          role="group"
          aria-label={t("marketplace:discover.categories")}
          className="flex flex-wrap gap-1.5"
        >
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
              {c === null ? t("marketplace:discover.allCategories") : categoryLabel(c)}
            </Button>
          ))}
        </div>
      )}
      {shown.length === 0 ? (
        <EmptyState variant="plain" title={t("marketplace:discover.noMatch.title")}>
          {category === null
            ? t("marketplace:discover.noMatch.text")
            : t("marketplace:discover.noMatch.inCategory", { category: categoryLabel(category) })}
        </EmptyState>
      ) : (
        <ul className="grid divide-y border-y" aria-label={t("marketplace:discover.list")}>
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
                      {item.latest && (
                        <Badge variant="outline">
                          {t("marketplace:badge.latestVersion", { version: item.latest })}
                        </Badge>
                      )}
                      {item.installed ? (
                        <>
                          <Badge variant="secondary">
                            {t("marketplace:badge.installedVersion", { version: item.installed })}
                          </Badge>
                          <Badge variant="outline">{t("marketplace:badge.optional")}</Badge>
                        </>
                      ) : (
                        <Badge variant="outline">{t("marketplace:badge.available")}</Badge>
                      )}
                      {updated.has(item.id) && <Badge>{t("marketplace:badge.update")}</Badge>}
                      {!item.installed && blocked && (
                        <Badge variant={blocked.tone}>
                          {blocked.label === "withdrawn"
                            ? t("marketplace:badge.withdrawn")
                            : t("marketplace:badge.incompatible")}
                        </Badge>
                      )}
                      {item.deprecated && (
                        <Badge variant="outline">{t("marketplace:badge.deprecated")}</Badge>
                      )}
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
