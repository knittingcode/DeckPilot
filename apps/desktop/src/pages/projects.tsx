import { Check, ChevronLeft, ChevronRight, Plus, Trash2, X } from "lucide-react"
import { useState } from "react"

import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useAsync } from "@/hooks/use-async"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import type { MachineConfig, ProfileView } from "@/lib/types"

export function ProjectsPage() {
  return (
    <Page title="Projects" description="Project context of the active profile">
      <RequireProfile>{(profileId) => <ProjectsView profileId={profileId} />}</RequireProfile>
    </Page>
  )
}

function ProjectsView({ profileId }: { profileId: string }) {
  const { revision, refresh } = useAppContext()
  const { state, reload } = useAsync(
    () => Promise.all([api.getProfile(profileId), api.getMachineConfig()]),
    `${profileId}:${revision}`,
  )
  const [error, setError] = useState<string | null>(null)

  const run = async (operation: () => Promise<unknown>) => {
    setError(null)
    try {
      await operation()
      reload()
      refresh()
    } catch (e) {
      setError(errorMessage(e))
    }
  }

  if (state.status === "loading") return <Loading />
  if (state.status === "error") return <ErrorNotice error={state.error} />
  const [view, machine] = state.data

  return (
    <>
      {error && <ErrorNotice error={error} />}
      <ProjectList view={view} machine={machine} run={run} />
      <NewProjectForm profileId={profileId} run={run} />
    </>
  )
}

function ProjectList({
  view,
  machine,
  run,
}: {
  view: ProfileView
  machine: MachineConfig
  run: (operation: () => Promise<unknown>) => Promise<void>
}) {
  const { profile, activeProjectId } = view
  const bound = new Set(machine.bindings.map((binding) => binding.key))

  return (
    <Card>
      <CardHeader>
        <CardTitle>Project cycle</CardTitle>
        <CardDescription>
          Projects are selected in this order and wrap around at the end. Switching projects only
          changes the active context; it does not touch repositories, containers or applications.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-4">
        {profile.projects.length === 0 ? (
          <EmptyState title="No projects in this profile" />
        ) : (
          <>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => void run(() => api.cycleProject(profile.id, "previous"))}
              >
                <ChevronLeft /> Previous
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => void run(() => api.cycleProject(profile.id, "next"))}
              >
                Next <ChevronRight />
              </Button>
            </div>
            <ol className="grid gap-2">
              {profile.projects.map((project, index) => {
                const active = project.id === activeProjectId
                const resources = Object.entries(project.resources)
                return (
                  <li
                    key={project.id}
                    className="flex items-start gap-3 rounded-lg border p-3 data-[active=true]:border-primary"
                    data-active={active}
                  >
                    <span className="w-6 pt-0.5 text-right text-sm text-muted-foreground tabular-nums">
                      {index + 1}.
                    </span>
                    <div className="grid flex-1 gap-1">
                      <div className="flex items-center gap-2">
                        <span className="font-mono text-sm font-semibold">{project.key}</span>
                        <span className="text-sm">{project.name}</span>
                        {active && <Badge>Active</Badge>}
                      </div>
                      {resources.length > 0 && (
                        <ul className="grid gap-0.5 text-xs text-muted-foreground">
                          {resources.map(([role, key]) => (
                            <li key={role} className="flex items-center gap-1">
                              {bound.has(key) ? (
                                <Check className="size-3 text-emerald-600" />
                              ) : (
                                <X className="size-3 text-destructive" />
                              )}
                              {role} → <span className="font-mono">{key}</span>
                              {!bound.has(key) && " (not bound on this machine)"}
                            </li>
                          ))}
                        </ul>
                      )}
                    </div>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={active}
                      onClick={() => void run(() => api.selectProject(profile.id, project.id))}
                    >
                      Select
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={`Delete ${project.key}`}
                      onClick={() => void run(() => api.deleteProject(profile.id, project.id))}
                    >
                      <Trash2 />
                    </Button>
                  </li>
                )
              })}
            </ol>
          </>
        )}
      </CardContent>
    </Card>
  )
}

interface ResourceRow {
  role: string
  key: string
}

function NewProjectForm({
  profileId,
  run,
}: {
  profileId: string
  run: (operation: () => Promise<unknown>) => Promise<void>
}) {
  const [key, setKey] = useState("")
  const [name, setName] = useState("")
  const [rows, setRows] = useState<ResourceRow[]>([])

  const updateRow = (index: number, patch: Partial<ResourceRow>) => {
    setRows((current) => current.map((row, i) => (i === index ? { ...row, ...patch } : row)))
  }

  const submit = (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    const resources = Object.fromEntries(
      rows.filter((row) => row.role.trim() || row.key.trim()).map((row) => [row.role, row.key]),
    )
    void run(async () => {
      await api.createProject(profileId, { key, name, resources })
      setKey("")
      setName("")
      setRows([])
    })
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>New project</CardTitle>
        <CardDescription>
          Resources map a role (for example <code>repository</code>) to a logical resource key. Each
          machine binds resource keys to concrete paths in Settings.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form className="grid max-w-xl gap-4" onSubmit={submit}>
          <div className="grid grid-cols-[8rem_1fr] gap-4">
            <div className="grid gap-2">
              <Label htmlFor="project-key">Key</Label>
              <Input
                id="project-key"
                value={key}
                onChange={(e) => {
                  setKey(e.target.value)
                }}
                placeholder="SHOP"
                required
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="project-name">Name</Label>
              <Input
                id="project-name"
                value={name}
                onChange={(e) => {
                  setName(e.target.value)
                }}
                placeholder="Optional display name"
              />
            </div>
          </div>
          <div className="grid gap-2">
            <Label>Resources</Label>
            {rows.map((row, index) => (
              <div key={index} className="grid grid-cols-[1fr_1fr_auto] gap-2">
                <Input
                  aria-label="Role"
                  value={row.role}
                  onChange={(e) => {
                    updateRow(index, { role: e.target.value })
                  }}
                  placeholder="repository"
                />
                <Input
                  aria-label="Resource key"
                  value={row.key}
                  onChange={(e) => {
                    updateRow(index, { key: e.target.value })
                  }}
                  placeholder="shop.repository"
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label="Remove resource"
                  onClick={() => {
                    setRows((current) => current.filter((_, i) => i !== index))
                  }}
                >
                  <X />
                </Button>
              </div>
            ))}
            <div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => {
                  setRows((current) => [...current, { role: "", key: "" }])
                }}
              >
                <Plus /> Add resource
              </Button>
            </div>
          </div>
          <div>
            <Button type="submit" disabled={!key.trim()}>
              Add project
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  )
}
