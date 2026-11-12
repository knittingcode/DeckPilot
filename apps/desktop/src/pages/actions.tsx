import { Plus, Search } from "lucide-react"
import { useMemo, useRef, useState } from "react"

import { ActionEditor } from "@/components/action-editor"
import { ActionTypeChooser } from "@/components/action-type-chooser"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useEditingProfile } from "@/hooks/use-profile"
import { useTitlebar } from "@/hooks/use-titlebar"
import { actionReferences, isDirect, referenceTarget } from "@/lib/action-references"
import { useAppContext } from "@/lib/app-context"
import { requestFocus } from "@/lib/focus"
import { moduleOf } from "@/lib/modules"
import { ActionIcon } from "@/components/action-icon"
import type { ActionInstance, ActionReference, ActionType, ProfileView } from "@/lib/types"
import { cn } from "@/lib/utils"

export function ActionsPage() {
  return (
    <Page
      title="Action Library"
      description="Actions you configured, ready for workflows and keys"
      profileScoped
    >
      <RequireProfile>{() => <ActionLibrary />}</RequireProfile>
    </Page>
  )
}

function ActionLibrary() {
  const { state, reload, profileId } = useEditingProfile()
  if (state.status === "loading") return <Loading />
  if (state.status === "error") return <ErrorNotice error={state.error} />
  if (!profileId) return null
  return <Library view={state.data} profileId={profileId} reload={reload} />
}

/**
 * The actions the user configured in this profile. Modules provide action
 * types; nothing appears here until the user creates an action: there are
 * no generated actions. A new action starts with its type, then its name
 * and parameters.
 */
function Library({
  view,
  profileId,
  reload,
}: {
  view: ProfileView
  profileId: string
  reload: () => void
}) {
  const { profiles, agent, refresh, modules, navigate, openSettings } = useAppContext()
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
        label: "New action",
        icon: Plus,
        placement: "primary",
        onSelect: startNew,
      },
      {
        id: "actions.search",
        label: "Search",
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
  const isAvailable = (id: string) => Boolean(moduleOf(modules, id)?.enabled)
  const categories = [
    ...new Set(view.profile.actions.map((a) => typeOf(a.actionType)?.category ?? "Other")),
  ].sort()

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
      const key = types.find((t) => t.id === action.actionType)?.category ?? "Other"
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
    <div className="grid gap-4 xl:grid-cols-[minmax(18rem,24rem)_1fr]">
      <Card className="xl:max-h-[calc(100vh-7rem)] xl:overflow-hidden">
        <CardHeader className="gap-3">
          <CardTitle>Actions</CardTitle>
          <div className="relative">
            <Search className="absolute top-2.5 left-2 size-4 text-muted-foreground" />
            <Input
              className="pl-8"
              ref={searchRef}
              placeholder="Search actions"
              aria-label="Search actions"
              value={query}
              onChange={(e) => {
                setQuery(e.target.value)
              }}
            />
          </div>
          <Select value={category} onValueChange={setCategory}>
            <SelectTrigger className="w-full" aria-label="Filter by category">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All categories</SelectItem>
              {categories.map((c) => (
                <SelectItem key={c} value={c}>
                  {c}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </CardHeader>
        <CardContent className="grid gap-4 xl:overflow-y-auto">
          {view.profile.actions.length === 0 && (
            <EmptyState title="No actions yet">
              Actions you create appear here, ready for workflows and device keys.
            </EmptyState>
          )}
          {view.profile.actions.length > 0 && groups.length === 0 && (
            <p className="text-sm text-muted-foreground">No actions match.</p>
          )}
          {groups.map(([group, actions]) => (
            <section key={group} className="grid min-w-0 gap-1" aria-label={group}>
              <h3 className="px-2 text-xs font-medium text-muted-foreground uppercase">{group}</h3>
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
                    {!isAvailable(action.actionType) && (
                      <Badge variant="outline" title="Its module is disabled">
                        Unavailable
                      </Badge>
                    )}
                    {used > 0 && (
                      <Badge variant="secondary" title={`Used ${used} time(s)`}>
                        {used}
                      </Badge>
                    )}
                  </button>
                )
              })}
            </section>
          ))}
        </CardContent>
      </Card>

      {current ? (
        <Card>
          <CardHeader>
            <CardTitle>{current.name}</CardTitle>
            <CardDescription>
              {typeOf(current.actionType)?.name ?? current.actionType}
            </CardDescription>
          </CardHeader>
          <CardContent>
            <ActionEditor
              key={current.id}
              profileId={profileId}
              modules={modules}
              projects={view.profile.projects}
              action={current}
              references={actionReferences(view.profile, current.id)}
              candidates={view.profile.actions}
              canRun={canRun}
              onSaved={changed}
              onDeleted={() => {
                setSelected(null)
                changed()
              }}
              onOpenReference={openReference}
            />
          </CardContent>
        </Card>
      ) : creating && newType ? (
        <Card>
          <CardHeader>
            <CardTitle>New action</CardTitle>
            <CardDescription>Step 2 of 2: name it and set its parameters</CardDescription>
          </CardHeader>
          <CardContent>
            <ActionEditor
              key={`new:${newType}`}
              profileId={profileId}
              modules={modules}
              projects={view.profile.projects}
              action={null}
              initialType={newType}
              references={[]}
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
          </CardContent>
        </Card>
      ) : creating ? (
        <Card>
          <CardHeader>
            <CardTitle>New action</CardTitle>
            <CardDescription>Step 1 of 2: choose what the action does</CardDescription>
          </CardHeader>
          <CardContent>
            <ActionTypeChooser
              modules={modules}
              onChoose={(type) => {
                setNewType(type.id)
              }}
              onOpenModules={() => {
                openSettings("modules")
              }}
            />
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent>
            {view.profile.actions.length === 0 ? (
              <EmptyState title="Create your first action">
                <p>
                  An action is one thing DeckPilot does, such as running a command or opening a URL.
                  Choose a type from your modules, give it a name and set its parameters. Then put
                  it on a device key or into a workflow.
                </p>
                <Button type="button" className="mt-3" onClick={startNew}>
                  <Plus /> Create action
                </Button>
              </EmptyState>
            ) : (
              <EmptyState title="No action selected">
                Choose an action on the left, or create a new one.
              </EmptyState>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  )
}
