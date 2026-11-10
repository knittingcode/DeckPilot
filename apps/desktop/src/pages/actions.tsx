import { Play, Trash2 } from "lucide-react"
import { useState } from "react"

import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { ParameterField } from "@/components/parameter-field"
import { RequireProfile } from "@/components/require-profile"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useAsync } from "@/hooks/use-async"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { buildParameters, describeValue, initialValue } from "@/lib/parameters"
import type { ActionResult, ActionType, ParameterValue, Profile } from "@/lib/types"

export function ActionsPage() {
  return (
    <Page title="Actions" description="Configured actions of the active profile">
      <RequireProfile>{(profileId) => <ActionsView profileId={profileId} />}</RequireProfile>
    </Page>
  )
}

function ActionsView({ profileId }: { profileId: string }) {
  const { revision, refresh } = useAppContext()
  const { state, reload } = useAsync(
    () => Promise.all([api.getProfile(profileId), api.listActionTypes()]),
    `${profileId}:${revision}`,
  )

  if (state.status === "loading") return <Loading />
  if (state.status === "error") return <ErrorNotice error={state.error} />
  const [view, types] = state.data
  const changed = () => {
    reload()
    refresh()
  }

  return (
    <>
      {view.issues.length > 0 && (
        <Alert variant="destructive">
          <AlertDescription>
            <ul className="list-disc pl-4">
              {view.issues.map((issue) => (
                <li key={issue}>{issue}</li>
              ))}
            </ul>
          </AlertDescription>
        </Alert>
      )}
      <ActionList profile={view.profile} types={types} onChange={changed} />
      <NewActionForm profileId={profileId} types={types} onCreated={changed} />
    </>
  )
}

function ActionList({
  profile,
  types,
  onChange,
}: {
  profile: Profile
  types: ActionType[]
  onChange: () => void
}) {
  const { agent } = useAppContext()
  const agentRunning = agent?.running ?? false
  const [results, setResults] = useState<Record<string, ActionResult>>({})
  const [error, setError] = useState<string | null>(null)
  const typeName = (id: string) => types.find((t) => t.id === id)?.name ?? id

  const runAction = async (actionId: string) => {
    setError(null)
    try {
      const result = await api.runAction(profile.id, actionId)
      setResults((current) => ({ ...current, [actionId]: result }))
      onChange()
    } catch (e) {
      setError(errorMessage(e))
    }
  }

  const remove = async (actionId: string) => {
    setError(null)
    try {
      await api.deleteAction(profile.id, actionId)
      onChange()
    } catch (e) {
      setError(errorMessage(e))
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Configured actions</CardTitle>
        <CardDescription>
          Actions are defined independently of device controls. “Run” executes an action in the
          background agent without pressing a key{agentRunning ? "" : " (start the agent first)"}.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-2">
        {error && <ErrorNotice error={error} />}
        {profile.actions.length === 0 ? (
          <EmptyState title="No actions configured" />
        ) : (
          profile.actions.map((action) => {
            const result = results[action.id]
            return (
              <div key={action.id} className="grid gap-2 rounded-lg border p-3">
                <div className="flex items-center gap-2">
                  <span className="font-medium">{action.name}</span>
                  <Badge variant="secondary">{typeName(action.actionType)}</Badge>
                  <div className="flex-1" />
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={!agentRunning}
                    onClick={() => void runAction(action.id)}
                  >
                    <Play /> Run
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={`Delete ${action.name}`}
                    onClick={() => void remove(action.id)}
                  >
                    <Trash2 />
                  </Button>
                </div>
                <dl className="grid grid-cols-[8rem_1fr] gap-x-4 text-xs">
                  {Object.entries(action.parameters).map(([key, value]) => (
                    <div key={key} className="contents">
                      <dt className="text-muted-foreground">{key}</dt>
                      <dd className="font-mono break-all">{describeValue(value)}</dd>
                    </div>
                  ))}
                </dl>
                {result && (
                  <p
                    className="rounded-md bg-muted px-3 py-2 font-mono text-xs data-[status=failed]:text-destructive"
                    data-status={result.status}
                  >
                    {result.status === "succeeded" ? "✓" : "✗"} {result.message}
                  </p>
                )}
              </div>
            )
          })
        )}
      </CardContent>
    </Card>
  )
}

function NewActionForm({
  profileId,
  types,
  onCreated,
}: {
  profileId: string
  types: ActionType[]
  onCreated: () => void
}) {
  const [typeId, setTypeId] = useState(types[0]?.id ?? "")
  const [name, setName] = useState("")
  const [values, setValues] = useState<Record<string, ParameterValue>>(() =>
    initialValues(types[0]),
  )
  const [error, setError] = useState<string | null>(null)
  const actionType = types.find((t) => t.id === typeId)

  const selectType = (id: string) => {
    setTypeId(id)
    setValues(initialValues(types.find((t) => t.id === id)))
  }

  const submit = async (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!actionType) return
    setError(null)
    try {
      await api.createAction(profileId, {
        name,
        actionType: actionType.id,
        parameters: buildParameters(actionType.parameters, values),
      })
      setName("")
      setValues(initialValues(actionType))
      onCreated()
    } catch (e) {
      setError(errorMessage(e))
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>New action</CardTitle>
        <CardDescription>Parameters are validated against the action type.</CardDescription>
      </CardHeader>
      <CardContent>
        <form className="grid max-w-xl gap-4" onSubmit={(e) => void submit(e)}>
          {error && <ErrorNotice error={error} />}
          <div className="grid gap-2">
            <Label htmlFor="action-type">Action type</Label>
            <Select value={typeId} onValueChange={selectType}>
              <SelectTrigger id="action-type" className="w-72">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {types.map((type) => (
                  <SelectItem key={type.id} value={type.id}>
                    {type.category} › {type.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {actionType && (
              <p className="text-xs text-muted-foreground">{actionType.description}</p>
            )}
          </div>
          <div className="grid gap-2">
            <Label htmlFor="action-name">Name</Label>
            <Input
              id="action-name"
              value={name}
              onChange={(e) => {
                setName(e.target.value)
              }}
              placeholder="Start Work"
              required
            />
          </div>
          {actionType?.parameters.map((spec) => {
            const value = values[spec.key] ?? initialValue(spec)
            return (
              <ParameterField
                key={`${actionType.id}:${spec.key}`}
                spec={spec}
                value={value}
                onChange={(next) => {
                  setValues((current) => ({ ...current, [spec.key]: next }))
                }}
              />
            )
          })}
          <div>
            <Button type="submit" disabled={!actionType || !name.trim()}>
              Create action
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  )
}

function initialValues(type: ActionType | undefined): Record<string, ParameterValue> {
  return Object.fromEntries((type?.parameters ?? []).map((spec) => [spec.key, initialValue(spec)]))
}
