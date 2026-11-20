import { ChevronDown, Search, Settings2 } from "lucide-react"
import { createElement, useState } from "react"
import { toast } from "sonner"

import { EmptyState } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { CAPABILITIES } from "@/lib/capabilities"
import { useAppContext } from "@/lib/app-context"
import { osLabel } from "@/lib/format"
import { type ModuleFilter, moduleMatches, moduleToggle } from "@/lib/modules"
import type { ModuleInfo } from "@/lib/types"
import { cn } from "@/lib/utils"
import { moduleIcon, moduleSettingsPages } from "@/modules"
import { type TranslationKey, useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"

const FILTERS: { value: ModuleFilter; label: TranslationKey }[] = [
  { value: "all", label: "settings:modules.filter.all" },
  { value: "enabled", label: "settings:modules.filter.enabled" },
  { value: "disabled", label: "settings:modules.filter.disabled" },
  { value: "unavailable", label: "settings:modules.filter.unavailable" },
]

/**
 * Feature modules, managed like plugins: search, filter, enable and disable,
 * with the reasons a module cannot be switched. Every module is built in;
 * DeckPilot does not load external code.
 */
export function ModulesSettings() {
  const { t } = useTranslation()
  const format = useFormat()
  const { modules, setModules, refresh } = useAppContext()
  const [query, setQuery] = useState("")
  const [filter, setFilter] = useState<ModuleFilter>("all")
  const [pending, setPending] = useState<string | null>(null)

  const toggle = (module: ModuleInfo, enabled: boolean) => {
    setPending(module.id)
    api
      .setModuleEnabled(module.id, enabled)
      .then(
        (next) => {
          setModules(next)
          refresh()
          toast.success(
            enabled
              ? t("settings:modules.toast.enabled", { module: module.name })
              : t("settings:modules.toast.disabled", { module: module.name }),
          )
        },
        (e: unknown) => {
          toastFailure(
            enabled
              ? t("settings:modules.toast.notEnabled", { module: module.name })
              : t("settings:modules.toast.notDisabled", { module: module.name }),
            e,
          )
        },
      )
      .finally(() => {
        setPending(null)
      })
  }

  const count = (value: ModuleFilter) => modules.filter((m) => moduleMatches(m, value, "")).length
  const shown = modules
    .filter((m) => moduleMatches(m, filter, query))
    .sort((a, b) => format.compare(a.name, b.name))
  const withSettings = new Set(moduleSettingsPages(modules).map((page) => page.module.id))

  return (
    <>
      <p className="text-sm text-muted-foreground">
        <span>{t("settings:modules.intro")}</span>
      </p>
      <div className="grid gap-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <div className="relative flex-1">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              type="search"
              placeholder={t("settings:modules.search")}
              aria-label={t("settings:modules.search")}
              className="h-8 pl-8"
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
            value={filter}
            onValueChange={(value) => {
              if (value) setFilter(value as ModuleFilter)
            }}
            aria-label={t("settings:modules.show")}
            className="flex-wrap"
          >
            {FILTERS.map((item) => (
              <ToggleGroupItem key={item.value} value={item.value} className="px-2.5">
                {t(item.label)}
                <span className="text-xs text-muted-foreground tabular-nums">
                  {count(item.value)}
                </span>
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        </div>
        {shown.length === 0 ? (
          <EmptyState variant="plain" title={t("settings:modules.emptyTitle")}>
            {query ? t("settings:modules.emptySearch") : t("settings:modules.emptyFilter")}
          </EmptyState>
        ) : (
          <ul className="grid divide-y border-y" aria-label={t("settings:pages.modules.title")}>
            {shown.map((module) => (
              <ModuleItem
                key={module.id}
                module={module}
                modules={modules}
                busy={pending !== null}
                hasSettings={withSettings.has(module.id)}
                onToggle={(enabled) => {
                  toggle(module, enabled)
                }}
              />
            ))}
          </ul>
        )}
      </div>
    </>
  )
}

function ModuleItem({
  module,
  modules,
  busy,
  hasSettings,
  onToggle,
}: {
  module: ModuleInfo
  modules: ModuleInfo[]
  busy: boolean
  hasSettings: boolean
  onToggle: (enabled: boolean) => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const { openSettings } = useAppContext()
  const [expanded, setExpanded] = useState(false)
  const { allowed, reason } = moduleToggle(module, modules)
  const id = `module-${module.id}`
  const dependents = modules.filter((m) => m.dependencies.includes(module.id))
  return (
    <li className="py-3" aria-labelledby={`${id}-name`}>
      <div className="flex items-start gap-3">
        <div
          className={cn(
            "flex size-8 shrink-0 items-center justify-center rounded-md bg-muted",
            module.enabled ? "text-foreground" : "text-muted-foreground",
          )}
          aria-hidden
        >
          {createElement(moduleIcon(module.id), { className: "size-4" })}
        </div>
        <div className="grid min-w-0 flex-1 gap-0.5">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span id={`${id}-name`} className="text-sm font-medium">
              {module.name}
            </span>
            <span className="text-xs text-muted-foreground">v{module.version}</span>
            <span className="text-xs text-muted-foreground">· {t("settings:modules.builtIn")}</span>
            {!module.supported && (
              <Badge variant="outline">{t("settings:modules.unsupported")}</Badge>
            )}
          </div>
          <p className="text-caption text-muted-foreground">{module.description}</p>
          <p className="text-xs text-muted-foreground">
            {t("settings:modules.actionTypeCount", { count: module.actionTypes.length })}
            {module.pages.length > 0 &&
              ` · ${t("settings:modules.pageCount", {
                count: module.pages.length,
                pages: format.list(
                  module.pages.map((p) => p.title),
                  "unit",
                ),
              })}`}
            {` · ${
              module.platforms.length === 0
                ? t("settings:modules.allPlatforms")
                : format.list(module.platforms.map(osLabel), "unit")
            }`}
          </p>
          {reason && (
            <p className="text-xs text-amber-700 dark:text-amber-400" id={`${id}-reason`}>
              {reason}
            </p>
          )}
        </div>
        <Switch
          checked={module.enabled}
          disabled={!allowed || busy}
          aria-label={
            module.enabled
              ? t("settings:modules.disable", { module: module.name })
              : t("settings:modules.enable", { module: module.name })
          }
          aria-describedby={reason ? `${id}-reason` : undefined}
          onCheckedChange={onToggle}
        />
      </div>
      <div className="mt-1.5 flex flex-wrap gap-1 pl-9">
        {hasSettings && (
          <Button
            variant="ghost"
            size="xs"
            onClick={() => {
              openSettings(`module:${module.id}`)
            }}
          >
            <Settings2 /> {t("settings:title")}
          </Button>
        )}
        <Button
          variant="ghost"
          size="xs"
          className="text-muted-foreground"
          aria-expanded={expanded}
          aria-controls={`${id}-details`}
          onClick={() => {
            setExpanded(!expanded)
          }}
        >
          <ChevronDown className={cn("transition-transform", expanded && "rotate-180")} />
          {expanded ? t("settings:modules.hideDetails") : t("settings:modules.details")}
        </Button>
      </div>
      {expanded && (
        <div
          id={`${id}-details`}
          className="mt-2 ml-11 grid gap-4 rounded-md bg-muted/50 p-3 text-sm sm:grid-cols-2"
        >
          <Detail title={t("settings:pages.permissions.title")}>
            {module.capabilities.length === 0 ? (
              <p className="text-muted-foreground">{t("settings:shared.none")}</p>
            ) : (
              <ul className="grid gap-0.5">
                {module.capabilities.map((capability) => (
                  <li key={capability}>{t(CAPABILITIES[capability].title)}</li>
                ))}
              </ul>
            )}
          </Detail>
          <Detail title={t("settings:modules.actionTypes")}>
            {module.actionTypes.length === 0 ? (
              <p className="text-muted-foreground">{t("settings:shared.none")}</p>
            ) : (
              <ul className="grid gap-0.5">
                {module.actionTypes.map((type) => (
                  <li key={type.id}>
                    {type.name}{" "}
                    <span className="font-mono text-xs text-muted-foreground">{type.id}</span>
                  </li>
                ))}
              </ul>
            )}
          </Detail>
          <Detail title={t("settings:modules.requires")}>
            <ModuleList ids={module.dependencies} modules={modules} />
          </Detail>
          <Detail title={t("settings:modules.requiredBy")}>
            <ModuleList ids={dependents.map((m) => m.id)} modules={modules} />
          </Detail>
          <Detail title={t("settings:modules.identifier")}>
            <span className="font-mono text-xs select-text">{module.id}</span>
          </Detail>
        </div>
      )}
    </li>
  )
}

function Detail({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="grid content-start gap-1">
      <h4 className="text-xs font-medium text-muted-foreground uppercase">{title}</h4>
      {children}
    </div>
  )
}

function ModuleList({ ids, modules }: { ids: string[]; modules: ModuleInfo[] }) {
  const { t } = useTranslation()
  if (ids.length === 0) {
    return <p className="text-muted-foreground">{t("settings:modules.noOtherModules")}</p>
  }
  return (
    <ul className="grid gap-0.5">
      {ids.map((id) => {
        const other = modules.find((m) => m.id === id)
        return (
          <li key={id}>
            {other?.name ?? id}{" "}
            <span className="text-xs text-muted-foreground">
              {!other
                ? t("settings:modules.state.notInstalled")
                : other.enabled
                  ? t("settings:modules.state.enabled")
                  : t("settings:modules.state.disabled")}
            </span>
          </li>
        )
      })}
    </ul>
  )
}
