import { ChevronDown, Info, Search, Settings2 } from "lucide-react"
import { createElement, useState } from "react"
import { toast } from "sonner"

import { EmptyState } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { api, errorMessage } from "@/lib/api"
import { CAPABILITIES } from "@/lib/capabilities"
import { useAppContext } from "@/lib/app-context"
import { osLabel } from "@/lib/format"
import { type ModuleFilter, moduleMatches, moduleToggle } from "@/lib/modules"
import type { ModuleInfo } from "@/lib/types"
import { cn } from "@/lib/utils"
import { moduleIcon, moduleSettingsPages } from "@/modules"

const FILTERS: { value: ModuleFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "enabled", label: "Enabled" },
  { value: "disabled", label: "Disabled" },
  { value: "unavailable", label: "Unavailable" },
]

/**
 * Feature modules, managed like plugins: search, filter, enable and disable,
 * with the reasons a module cannot be switched. Every module is built in;
 * DeckPilot does not load external code.
 */
export function ModulesSettings() {
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
          toast.success(`${module.name} ${enabled ? "enabled" : "disabled"}`)
        },
        (e: unknown) => toast.error(errorMessage(e)),
      )
      .finally(() => {
        setPending(null)
      })
  }

  const count = (value: ModuleFilter) => modules.filter((m) => moduleMatches(m, value, "")).length
  const shown = modules
    .filter((m) => moduleMatches(m, filter, query))
    .sort((a, b) => a.name.localeCompare(b.name))
  const withSettings = new Set(moduleSettingsPages(modules).map((page) => page.module.id))

  return (
    <>
      <p className="flex items-start gap-2 text-sm text-muted-foreground">
        <Info className="mt-0.5 size-4 shrink-0" />
        <span>
          Disabling a module hides its pages and action types. Actions, workflows and layouts that
          use it are kept and shown as unavailable until you enable it again. All modules are built
          into DeckPilot; installing modules from other sources is not supported.
        </span>
      </p>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="search"
            placeholder="Search modules"
            aria-label="Search modules"
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
          value={filter}
          onValueChange={(value) => {
            if (value) setFilter(value as ModuleFilter)
          }}
          aria-label="Show modules"
          className="flex-wrap"
        >
          {FILTERS.map((item) => (
            <ToggleGroupItem key={item.value} value={item.value} className="px-2.5">
              {item.label}
              <span className="text-xs text-muted-foreground tabular-nums">
                {count(item.value)}
              </span>
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
      {shown.length === 0 ? (
        <EmptyState title="No modules found">
          {query ? "No module matches your search." : "No module is in this state."}
        </EmptyState>
      ) : (
        <ul className="grid gap-3" aria-label="Modules">
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
  const { openSettings } = useAppContext()
  const [expanded, setExpanded] = useState(false)
  const { allowed, reason } = moduleToggle(module, modules)
  const id = `module-${module.id}`
  const dependents = modules.filter((m) => m.dependencies.includes(module.id))
  return (
    <li
      className={cn("rounded-xl border bg-card p-4", !module.enabled && "bg-muted/30")}
      aria-labelledby={`${id}-name`}
    >
      <div className="flex items-start gap-3">
        <div
          className={cn(
            "flex size-10 shrink-0 items-center justify-center rounded-lg border",
            module.enabled ? "bg-primary/10 text-primary" : "bg-muted text-muted-foreground",
          )}
          aria-hidden
        >
          {createElement(moduleIcon(module.id), { className: "size-5" })}
        </div>
        <div className="grid min-w-0 flex-1 gap-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span id={`${id}-name`} className="font-medium">
              {module.name}
            </span>
            <span className="text-xs text-muted-foreground">v{module.version}</span>
            <Badge variant="secondary">Built-in</Badge>
            {!module.supported && <Badge variant="outline">Not available on this system</Badge>}
          </div>
          <p className="text-sm text-muted-foreground">{module.description}</p>
          <p className="text-xs text-muted-foreground">
            {module.actionTypes.length} action type{module.actionTypes.length === 1 ? "" : "s"}
            {module.pages.length > 0 && ` · ${module.pages.map((p) => p.title).join(", ")} page`}
            {` · ${module.platforms.length === 0 ? "all platforms" : module.platforms.map(osLabel).join(", ")}`}
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
          aria-label={`${module.enabled ? "Disable" : "Enable"} ${module.name}`}
          aria-describedby={reason ? `${id}-reason` : undefined}
          onCheckedChange={onToggle}
        />
      </div>
      <div className="mt-3 flex flex-wrap gap-2 pl-13">
        {hasSettings && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              openSettings(`module:${module.id}`)
            }}
          >
            <Settings2 /> Settings
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          aria-expanded={expanded}
          aria-controls={`${id}-details`}
          onClick={() => {
            setExpanded(!expanded)
          }}
        >
          <ChevronDown className={cn("transition-transform", expanded && "rotate-180")} />
          {expanded ? "Hide details" : "Details"}
        </Button>
      </div>
      {expanded && (
        <div id={`${id}-details`} className="mt-3 grid gap-4 border-t pt-3 text-sm sm:grid-cols-2">
          <Detail title="Permissions">
            {module.capabilities.length === 0 ? (
              <p className="text-muted-foreground">None</p>
            ) : (
              <ul className="grid gap-0.5">
                {module.capabilities.map((capability) => (
                  <li key={capability}>{CAPABILITIES[capability].title}</li>
                ))}
              </ul>
            )}
          </Detail>
          <Detail title="Action types">
            {module.actionTypes.length === 0 ? (
              <p className="text-muted-foreground">None</p>
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
          <Detail title="Requires">
            <ModuleList ids={module.dependencies} modules={modules} />
          </Detail>
          <Detail title="Required by">
            <ModuleList ids={dependents.map((m) => m.id)} modules={modules} />
          </Detail>
          <Detail title="Identifier">
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
  if (ids.length === 0) return <p className="text-muted-foreground">No other modules</p>
  return (
    <ul className="grid gap-0.5">
      {ids.map((id) => {
        const other = modules.find((m) => m.id === id)
        return (
          <li key={id}>
            {other?.name ?? id}{" "}
            <span className="text-xs text-muted-foreground">
              {!other ? "not installed" : other.enabled ? "enabled" : "disabled"}
            </span>
          </li>
        )
      })}
    </ul>
  )
}
