import { Plus, Search } from "lucide-react"
import { useMemo, useState } from "react"

import { ActionEditor } from "@/components/action-editor"
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
import { useAsync } from "@/hooks/use-async"
import { useEditingProfile } from "@/hooks/use-profile"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { ActionIcon } from "@/components/action-icon"
import type { ActionInstance, ActionType, ProfileView } from "@/lib/types"
import { cn } from "@/lib/utils"

export function ActionsPage() {
  return (
    <Page title="Action Library" description="Configure actions before assigning them to keys">
      <RequireProfile>{() => <ActionLibrary />}</RequireProfile>
    </Page>
  )
}

function ActionLibrary() {
  const { state, reload, profileId } = useEditingProfile()
  const types = useAsync(() => api.listActionTypes(), "types").state
  if (state.status === "loading" || types.status === "loading") return <Loading />
  if (state.status === "error") return <ErrorNotice error={state.error} />
  if (types.status === "error") return <ErrorNotice error={types.error} />
  if (!profileId) return null
  return <Library view={state.data} types={types.data} profileId={profileId} reload={reload} />
}

function Library({
  view,
  types,
  profileId,
  reload,
}: {
  view: ProfileView
  types: ActionType[]
  profileId: string
  reload: () => void
}) {
  const { profiles, agent, refresh } = useAppContext()
  const [query, setQuery] = useState("")
  const [category, setCategory] = useState("all")
  const [selected, setSelected] = useState<string | null>(view.profile.actions[0]?.id ?? "new")
  const typeOf = (id: string) => types.find((t) => t.id === id)
  const categories = [...new Set(types.map((t) => t.category))]

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
          <div className="flex items-center gap-2">
            <CardTitle className="flex-1">Actions</CardTitle>
            <Button
              size="sm"
              onClick={() => {
                setSelected("new")
              }}
            >
              <Plus /> New
            </Button>
          </div>
          <div className="relative">
            <Search className="absolute top-2.5 left-2 size-4 text-muted-foreground" />
            <Input
              className="pl-8"
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
            <EmptyState title="No actions yet">Create your first action.</EmptyState>
          )}
          {view.profile.actions.length > 0 && groups.length === 0 && (
            <p className="text-sm text-muted-foreground">No actions match.</p>
          )}
          {groups.map(([group, actions]) => (
            <section key={group} className="grid gap-1" aria-label={group}>
              <h3 className="px-2 text-xs font-medium text-muted-foreground uppercase">{group}</h3>
              {actions.map((action) => {
                const used = view.actionUsages[action.id]?.length ?? 0
                return (
                  <button
                    key={action.id}
                    type="button"
                    className={cn(
                      "flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
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

      <Card>
        <CardHeader>
          <CardTitle>{current ? current.name : "New action"}</CardTitle>
          <CardDescription>
            {current
              ? (typeOf(current.actionType)?.name ?? current.actionType)
              : "Choose a type and configure its parameters"}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ActionEditor
            key={current?.id ?? "new"}
            profileId={profileId}
            types={types}
            action={current ?? null}
            usages={current ? (view.actionUsages[current.id] ?? []) : []}
            canRun={canRun}
            onSaved={changed}
            onDeleted={() => {
              setSelected(null)
              changed()
            }}
          />
        </CardContent>
      </Card>
    </div>
  )
}
