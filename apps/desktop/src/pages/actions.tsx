import { Plus, Search } from "lucide-react"
import { useMemo, useRef, useState } from "react"

import { ActionEditor } from "@/components/action-editor"
import { ActionTypeChooser } from "@/components/action-type-chooser"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Panel } from "@/components/settings-layout"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useEditingProfile } from "@/hooks/use-profile"
import { useTranslation } from "@/i18n"
import { useTitlebar } from "@/hooks/use-titlebar"
import { actionReferences, isDirect, referenceTarget } from "@/lib/action-references"
import { useActionModules, useAppContext } from "@/lib/app-context"
import { requestFocus } from "@/lib/focus"
import { typeAvailability } from "@/lib/plugin-actions"
import { ActionIcon } from "@/components/action-icon"
import type { ActionInstance, ActionReference, ActionType, ProfileView } from "@/lib/types"
import { cn } from "@/lib/utils"

export function ActionsPage() {
  const { t } = useTranslation()
  return (
    <Page
      title={t("navigation:pages.actions")}
      description={t("actions:library.description")}
      profileScoped
    >
      <RequireProfile>{() => <ActionLibrary />}</RequireProfile>
    </Page>
  )
}

/** The category of actions whose type names none. */
const OTHER = "Other"

/** A plugin whose actions the library shows alone (its toolbar view). */
export interface LibraryPlugin {
  id: string
  name: string
}

/**
 * The Action Library of the profile being edited, without a page around
 * it; with `plugin`, only that plugin's actions (a plugin's "actions"
 * view, drawn by the app).
 */
export function ActionLibraryView({ plugin }: { plugin?: LibraryPlugin }) {
  return <RequireProfile>{() => <ActionLibrary plugin={plugin} />}</RequireProfile>
}

function ActionLibrary({ plugin }: { plugin?: LibraryPlugin }) {
  const { state, reload, profileId } = useEditingProfile()
  if (state.status === "loading") return <Loading />
  if (state.status === "error") return <ErrorNotice error={state.error} />
  if (!profileId) return null
  return <Library view={state.data} profileId={profileId} reload={reload} plugin={plugin} />
}

/**
 * The actions the user configured in this profile. Modules provide action
 * types; nothing appears here until the user creates an action: there are
 * no generated actions. A new action starts with its type, then its name
 * and parameters.
 */
function Library({
  view: profileView,
  profileId,
  reload,
  plugin,
}: {
  view: ProfileView
  profileId: string
  reload: () => void
  plugin?: LibraryPlugin
}) {
  const { t } = useTranslation()
  // A plugin's view: its actions only (types `<plugin id>.<action>`).
  const view = useMemo(
    () =>
      plugin
        ? {
            ...profileView,
            profile: {
              ...profileView.profile,
              actions: profileView.profile.actions.filter((a) =>
                a.actionType.startsWith(`${plugin.id}.`),
              ),
            },
          }
        : profileView,
    [profileView, plugin],
  )
  const { profiles, agent, refresh, navigate, openSettings } = useAppContext()
  // Built-in modules and, in builds with plugins, the installed plugins.
  const modules = useActionModules()
  const [query, setQuery] = useState("")
  const [category, setCategory] = useState("all")
  const [selected, setSelected] = useState<string | null>(view.profile.actions[0]?.id ?? null)
  // For a new action: the type chosen in the first step.
  const [newType, setNewType] = useState<string | null>(null)
  const startNew = () => {
    setSelected("new")
    setNewType(null)
  }
  const searchRef = useRef<HTMLInputElement>(null)
  useTitlebar({
    actions: [
      {
        id: "actions.new",
        label: t("actions:library.new"),
        icon: Plus,
        placement: "primary",
        onSelect: startNew,
      },
      {
        id: "actions.search",
        label: t("actions:library.search"),
        icon: Search,
        onSelect: () => {
          searchRef.current?.focus()
        },
      },
    ],
  })
  // Types of disabled modules are known too, so their actions keep a name.
  const types: ActionType[] = useMemo(() => modules.flatMap((m) => m.actionTypes), [modules])
  const typeOf = (id: string) => types.find((t) => t.id === id)
  const availability = (id: string) => typeAvailability(modules, id)
  const categories = [
    ...new Set(view.profile.actions.map((a) => typeOf(a.actionType)?.category ?? OTHER)),
  ].sort()
  // The fallback category is the app's; plugin categories are shown as they are.
  const categoryLabel = (c: string) => (c === OTHER ? t("actions:library.otherCategory") : c)

  const groups = useMemo(() => {
    const needle = query.trim().toLowerCase()
    const matches = view.profile.actions.filter((action) => {
      const type = types.find((t) => t.id === action.actionType)
      const haystack =
        `${action.name} ${action.description ?? ""} ${type?.name ?? ""}`.toLowerCase()
      return (
        (!needle || haystack.includes(needle)) &&
        (category === "all" || type?.category === category)
      )
    })
    const byCategory = new Map<string, ActionInstance[]>()
    for (const action of matches) {
      const key = types.find((t) => t.id === action.actionType)?.category ?? OTHER
      byCategory.set(key, [...(byCategory.get(key) ?? []), action])
    }
    return [...byCategory.entries()].sort(([a], [b]) => a.localeCompare(b))
  }, [view.profile.actions, types, query, category])

  const current =
    selected && selected !== "new" ? view.profile.actions.find((a) => a.id === selected) : null
  const creating = selected === "new"
  const openReference = (reference: ActionReference) => {
    const target = referenceTarget(reference)
    requestFocus(target.focus)
    navigate(target.page)
  }
  const canRun = Boolean(agent?.running) && profiles.activeProfileId === profileId
  const changed = (action?: ActionInstance) => {
    if (action) setSelected(action.id)
    reload()
    refresh()
  }

  return (
    <div className="grid gap-6 lg:grid-cols-[minmax(14rem,18rem)_1fr]">
      <div
        role="group"
        aria-labelledby="action-list-title"
        className="grid content-start gap-3 lg:sticky lg:top-0 lg:max-h-[calc(100vh-8rem)] lg:grid-rows-[auto_1fr]"
      >
        <div className="grid gap-2">
          <h2 id="action-list-title" className="text-sm leading-6 font-semibold">
            {t("actions:library.listTitle")}
          </h2>
          <div className="relative">
            <Search className="absolute top-1/2 left-2 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              className="h-8 pl-8"
              ref={searchRef}
              placeholder={t("actions:library.searchPlaceholder")}
              aria-label={t("actions:library.searchPlaceholder")}
              value={query}
              onChange={(e) => {
                setQuery(e.target.value)
              }}
            />
          </div>
          <Select value={category} onValueChange={setCategory}>
            <SelectTrigger
              size="sm"
              className="w-full"
              aria-label={t("actions:library.filterCategory")}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{t("actions:library.allCategories")}</SelectItem>
              {categories.map((c) => (
                <SelectItem key={c} value={c}>
                  {categoryLabel(c)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="relative -mx-2 grid content-start gap-3 border-t px-2 pt-3 lg:overflow-y-auto">
          {view.profile.actions.length === 0 && (
            <EmptyState
              variant="plain"
              title={
                plugin
                  ? t("actions:library.emptyPlugin", { plugin: plugin.name })
                  : t("actions:library.empty")
              }
            >
              {t("actions:library.emptyHint")}
            </EmptyState>
          )}
          {view.profile.actions.length > 0 && groups.length === 0 && (
            <p className="text-sm text-muted-foreground">{t("actions:library.noMatches")}</p>
          )}
          {groups.map(([group, actions]) => (
            <section key={group} className="grid min-w-0 gap-1" aria-label={categoryLabel(group)}>
              <h3 className="px-2 text-xs font-medium text-muted-foreground uppercase">
                {categoryLabel(group)}
              </h3>
              {actions.map((action) => {
                const used = actionReferences(view.profile, action.id).filter(isDirect).length
                return (
                  <button
                    key={action.id}
                    type="button"
                    className={cn(
                      "flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                      selected === action.id && "bg-accent",
                    )}
                    onClick={() => {
                      setSelected(action.id)
                    }}
                  >
                    <ActionIcon name={action.icon} className="size-4 shrink-0" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate">{action.name}</span>
                      <span className="block truncate text-xs text-muted-foreground">
                        {typeOf(action.actionType)?.name ?? action.actionType}
                      </span>
                    </span>
                    {!availability(action.actionType).available && (
                      <Badge
                        variant="outline"
                        title={
                          availability(action.actionType).reason ??
                          t("actions:library.moduleDisabled")
                        }
                      >
                        {t("actions:library.unavailable")}
                      </Badge>
                    )}
                    {used > 0 && (
                      <Badge
                        variant="secondary"
                        title={t("actions:library.usedTimes", { count: used })}
                      >
                        {used}
                      </Badge>
                    )}
                  </button>
                )
              })}
            </section>
          ))}
        </div>
      </div>

      {current ? (
        <Panel
          className="content-start"
          title={current.name}
          description={typeOf(current.actionType)?.name ?? current.actionType}
        >
          <ActionEditor
            key={current.id}
            profileId={profileId}
            modules={modules}
            projects={view.profile.projects}
            workflows={view.profile.workflows}
            action={current}
            references={actionReferences(view.profile, current.id)}
            candidates={profileView.profile.actions}
            canRun={canRun}
            onSaved={changed}
            onDeleted={() => {
              setSelected(null)
              changed()
            }}
            onOpenReference={openReference}
          />
        </Panel>
      ) : creating && newType ? (
        <Panel
          className="content-start"
          title={t("actions:library.new")}
          description={t("actions:library.step2")}
        >
          <ActionEditor
            key={`new:${newType}`}
            profileId={profileId}
            modules={modules}
            projects={view.profile.projects}
            workflows={view.profile.workflows}
            action={null}
            initialType={newType}
            references={[]}
            candidates={profileView.profile.actions}
            canRun={false}
            onSaved={(action) => {
              setNewType(null)
              changed(action)
            }}
            onDeleted={() => {
              setSelected(null)
            }}
            onChangeType={() => {
              setNewType(null)
            }}
          />
        </Panel>
      ) : creating ? (
        <Panel
          className="content-start"
          title={t("actions:library.new")}
          description={t("actions:library.step1")}
        >
          <ActionTypeChooser
            modules={modules}
            onChoose={(type) => {
              setNewType(type.id)
            }}
            onOpenModules={() => {
              openSettings("modules")
            }}
          />
        </Panel>
      ) : (
        <div className="grid content-start">
          {view.profile.actions.length === 0 ? (
            <EmptyState title={t("actions:library.first.title")}>
              <p>{t("actions:library.first.body")}</p>
              <Button type="button" className="mt-3" onClick={startNew}>
                <Plus /> {t("actions:library.first.create")}
              </Button>
            </EmptyState>
          ) : (
            <EmptyState title={t("actions:library.noSelection.title")}>
              {t("actions:library.noSelection.body")}
            </EmptyState>
          )}
        </div>
      )}
    </div>
  )
}
