import { Search } from "lucide-react"
import { useMemo, useState } from "react"

import { EmptyState } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useTranslation } from "@/i18n"
import { creatableTypes } from "@/lib/modules"
import { pluginTypeUnavailable } from "@/lib/plugin-actions"
import type { ActionType, ModuleInfo } from "@/lib/types"

/**
 * The first step of creating an action: one of the action types the
 * enabled modules offer. Deprecated types are not offered. Installed
 * plugins (builds with plugins only) are listed under their name; their
 * actions that cannot run now are shown with the reason and cannot be
 * chosen.
 */
export function ActionTypeChooser({
  modules,
  onChoose,
  onOpenModules,
}: {
  modules: ModuleInfo[]
  onChoose: (type: ActionType) => void
  onOpenModules: () => void
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  const groups = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return modules
      .filter((m) => m.plugin || (m.enabled && m.supported))
      .map((m) => ({
        module: m,
        types: creatableTypes(m.actionTypes).filter((type) =>
          `${type.name} ${type.description} ${type.category} ${m.name}`
            .toLowerCase()
            .includes(needle),
        ),
      }))
      .filter((g) => g.types.length > 0)
  }, [modules, query])
  const offered = modules.some((m) =>
    m.plugin
      ? creatableTypes(m.actionTypes).some((type) => !pluginTypeUnavailable(m, type.id))
      : m.enabled && m.supported && creatableTypes(m.actionTypes).length > 0,
  )

  if (!offered) {
    return (
      <EmptyState title={t("actions:typeChooser.none.title")}>
        <p>{t("actions:typeChooser.none.body")}</p>
        <Button type="button" variant="outline" className="mt-3" onClick={onOpenModules}>
          {t("actions:typeChooser.none.openSettings")}
        </Button>
      </EmptyState>
    )
  }

  return (
    <div className="grid gap-4" role="group" aria-label={t("actions:typeChooser.label")}>
      <div className="relative">
        <Search className="absolute top-2.5 left-2 size-4 text-muted-foreground" />
        <Input
          className="pl-8"
          placeholder={t("actions:typeChooser.search")}
          aria-label={t("actions:typeChooser.search")}
          value={query}
          autoFocus
          onChange={(e) => {
            setQuery(e.target.value)
          }}
        />
      </div>
      {groups.length === 0 && (
        <p className="text-sm text-muted-foreground">{t("actions:typeChooser.noMatches")}</p>
      )}
      {groups.map(({ module, types }) => (
        <section key={module.id} className="grid gap-1" aria-label={module.name}>
          <h3 className="px-2 text-xs font-medium text-muted-foreground uppercase">
            {module.name}
            {module.plugin && (
              <span className="ml-1 normal-case">· {t("actions:typeChooser.plugin")}</span>
            )}
          </h3>
          {types.map((type) => {
            const unavailable = pluginTypeUnavailable(module, type.id)
            return (
              <button
                key={type.id}
                type="button"
                disabled={unavailable !== null}
                className="grid rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:bg-transparent"
                onClick={() => {
                  onChoose(type)
                }}
              >
                <span>{type.name}</span>
                <span className="text-xs text-muted-foreground">{type.description}</span>
                {unavailable !== null && (
                  <span className="text-xs text-muted-foreground">
                    {t("actions:typeChooser.unavailable", { reason: unavailable })}
                  </span>
                )}
              </button>
            )
          })}
        </section>
      ))}
    </div>
  )
}
