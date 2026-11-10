import { Ban, Copy, Play, Save, Trash2 } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { IconPicker } from "@/components/icon-picker"
import { ErrorNotice } from "@/components/page"
import { ParameterField } from "@/components/parameter-field"
import { UsageList } from "@/components/usage-list"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Separator } from "@/components/ui/separator"
import { Textarea } from "@/components/ui/textarea"
import { api, errorMessage } from "@/lib/api"
import { moduleOf } from "@/lib/modules"
import { buildParameters, initialValue, missingRequired } from "@/lib/parameters"
import type {
  ActionInstance,
  ActionResult,
  ActionType,
  ModuleInfo,
  ParameterValue,
  Project,
  Usage,
} from "@/lib/types"

function valuesFor(type: ActionType | undefined, action: ActionInstance | null) {
  return Object.fromEntries(
    (type?.parameters ?? []).map((spec) => [
      spec.key,
      action?.parameters[spec.key] ?? initialValue(spec),
    ]),
  ) as Record<string, ParameterValue>
}

/**
 * Creates or edits one action instance. A new action starts by choosing a
 * module and one of its action types; the form is then generated from the
 * type's parameter schema, and the backend validates again on save.
 *
 * Actions whose module is disabled stay visible but cannot be edited or
 * run until the module is enabled again. They can still be deleted.
 */
export function ActionEditor({
  profileId,
  modules,
  projects,
  action,
  usages,
  canRun,
  onSaved,
  onDeleted,
}: {
  profileId: string
  modules: ModuleInfo[]
  /** Projects of the profile, for project parameters. */
  projects: Project[]
  action: ActionInstance | null
  usages: Usage[]
  canRun: boolean
  onSaved: (action: ActionInstance) => void
  onDeleted: () => void
}) {
  const usable = modules.filter((m) => m.enabled && m.supported && m.actionTypes.length > 0)
  const owner = action ? moduleOf(modules, action.actionType) : undefined
  const unavailable = Boolean(action) && !owner?.enabled
  const [moduleId, setModuleId] = useState(owner?.id ?? (usable.length === 1 ? usable[0]?.id : ""))
  const module = modules.find((m) => m.id === moduleId)
  const types: ActionType[] = module?.actionTypes ?? []
  // A module with a single action type needs no second choice.
  const onlyType = (m: ModuleInfo | undefined) =>
    m?.actionTypes.length === 1 ? (m.actionTypes[0]?.id ?? "") : ""
  const [typeId, setTypeId] = useState(action?.actionType ?? onlyType(module))
  const type = types.find((t) => t.id === typeId)
  const [name, setName] = useState(action?.name ?? "")
  const [description, setDescription] = useState(action?.description ?? "")
  const [icon, setIcon] = useState<string | null>(action?.icon ?? null)
  const [label, setLabel] = useState(action?.label ?? "")
  const [values, setValues] = useState(() => valuesFor(type, action))
  const [submitted, setSubmitted] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<ActionResult | null>(null)

  const missing = type ? missingRequired(type.parameters, values) : []
  const nameMissing = !name.trim()

  const save = async (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    setSubmitted(true)
    if (!type || unavailable || nameMissing || missing.length > 0) return
    setError(null)
    const fields = {
      name,
      description,
      icon,
      label: label.trim() ? label : null,
      parameters: buildParameters(type.parameters, values),
    }
    try {
      const saved = action
        ? await api.updateAction(profileId, { ...action, ...fields })
        : await api.createAction(profileId, { ...fields, actionType: type.id })
      toast.success(action ? "Action saved" : "Action created")
      onSaved(saved)
    } catch (e) {
      setError(errorMessage(e))
    }
  }

  const duplicate = async () => {
    if (!action) return
    try {
      const copy = await api.duplicateAction(profileId, action.id, `${action.name} (copy)`)
      toast.success("Action duplicated")
      onSaved(copy)
    } catch (e) {
      toast.error(errorMessage(e))
    }
  }

  const remove = async () => {
    if (!action) return
    try {
      await api.deleteAction(profileId, action.id)
      toast.success("Action deleted")
      onDeleted()
    } catch (e) {
      toast.error(errorMessage(e))
    }
  }

  const run = async () => {
    if (!action) return
    try {
      setResult(await api.runAction(profileId, action.id))
    } catch (e) {
      toast.error(errorMessage(e))
    }
  }

  return (
    <form className="grid gap-4" onSubmit={(e) => void save(e)} noValidate>
      {error && <ErrorNotice error={error} />}
      {unavailable && (
        <Alert>
          <Ban />
          <AlertTitle>Unavailable</AlertTitle>
          <AlertDescription>
            {owner
              ? `The ${owner.name} module is disabled. Enable it in Settings to edit or run this action; it is kept until then.`
              : `No installed module provides “${action?.actionType ?? ""}”. The action is kept but cannot run.`}
          </AlertDescription>
        </Alert>
      )}
      {action ? (
        <p className="text-sm text-muted-foreground">
          {owner?.name ?? "Unknown module"} › {type?.name ?? action.actionType}
          {type && <span className="block text-xs">{type.description}</span>}
        </p>
      ) : (
        <div className="grid items-start gap-4 sm:grid-cols-2">
          <div className="grid gap-2">
            <Label htmlFor="action-module">Module</Label>
            <Select
              value={moduleId}
              onValueChange={(id) => {
                const chosen = modules.find((m) => m.id === id)
                const next = onlyType(chosen)
                setModuleId(id)
                setTypeId(next)
                setValues(
                  valuesFor(
                    chosen?.actionTypes.find((t) => t.id === next),
                    null,
                  ),
                )
              }}
            >
              <SelectTrigger id="action-module" className="w-full">
                <SelectValue placeholder="Choose a module" />
              </SelectTrigger>
              <SelectContent>
                {usable.map((m) => (
                  <SelectItem key={m.id} value={m.id}>
                    {m.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {module && <p className="text-xs text-muted-foreground">{module.description}</p>}
          </div>
          <div className="grid gap-2">
            <Label htmlFor="action-type">Action type</Label>
            <Select
              value={typeId}
              disabled={!module}
              onValueChange={(id) => {
                setTypeId(id)
                setValues(
                  valuesFor(
                    types.find((t) => t.id === id),
                    null,
                  ),
                )
              }}
            >
              <SelectTrigger id="action-type" className="w-full">
                <SelectValue placeholder={module ? "Choose a type" : "Choose a module first"} />
              </SelectTrigger>
              <SelectContent>
                {types.map((t) => (
                  <SelectItem key={t.id} value={t.id}>
                    {t.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {type && <p className="text-xs text-muted-foreground">{type.description}</p>}
            {submitted && !type && (
              <p className="text-xs text-destructive">Choose an action type.</p>
            )}
          </div>
        </div>
      )}

      <div className="grid gap-4 sm:grid-cols-2">
        <div className="grid gap-2">
          <Label htmlFor="action-name">Name</Label>
          <Input
            id="action-name"
            value={name}
            aria-invalid={submitted && nameMissing}
            onChange={(e) => {
              setName(e.target.value)
            }}
            placeholder="Git Status — Active Project"
          />
          {submitted && nameMissing && <p className="text-xs text-destructive">Enter a name.</p>}
        </div>
        <div className="grid gap-2">
          <Label htmlFor="action-icon">Icon</Label>
          <IconPicker id="action-icon" value={icon} onChange={setIcon} />
        </div>
      </div>

      <div className="grid gap-2">
        <Label htmlFor="action-label">Key label</Label>
        <Input
          id="action-label"
          value={label}
          maxLength={32}
          onChange={(e) => {
            setLabel(e.target.value)
          }}
          placeholder="Defaults to the name"
        />
        <p className="text-xs text-muted-foreground">
          Use {"{project.key}"} or {"{project.name}"} to show the active project on the key.
        </p>
      </div>

      <div className="grid gap-2">
        <Label htmlFor="action-description">Description</Label>
        <Textarea
          id="action-description"
          value={description}
          rows={2}
          onChange={(e) => {
            setDescription(e.target.value)
          }}
          placeholder="Optional"
        />
      </div>

      {type && type.parameters.length > 0 && (
        <>
          <Separator />
          <div className="grid gap-4">
            {type.parameters.map((spec) => (
              <div key={`${type.id}:${spec.key}`} className="grid gap-1">
                <ParameterField
                  spec={spec}
                  value={values[spec.key] ?? initialValue(spec)}
                  projects={projects}
                  onChange={(next) => {
                    setValues((current) => ({ ...current, [spec.key]: next }))
                  }}
                />
                {submitted && missing.includes(spec.key) && (
                  <p className="text-xs text-destructive">{spec.label} is required.</p>
                )}
              </div>
            ))}
          </div>
        </>
      )}

      <div className="flex flex-wrap gap-2">
        <Button type="submit" disabled={unavailable}>
          <Save /> {action ? "Save" : "Create action"}
        </Button>
        {action && (
          <>
            <Button
              type="button"
              variant="outline"
              disabled={unavailable}
              onClick={() => void duplicate()}
            >
              <Copy /> Duplicate
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={!canRun || unavailable}
              title={canRun ? undefined : "Runs in the agent for the active profile"}
              onClick={() => void run()}
            >
              <Play /> Run
            </Button>
            <ConfirmDialog
              trigger={
                <Button type="button" variant="ghost" disabled={usages.length > 0}>
                  <Trash2 /> Delete
                </Button>
              }
              title={`Delete “${action.name}”?`}
              description="The action is removed from this profile. This cannot be undone."
              onConfirm={() => void remove()}
            />
          </>
        )}
      </div>

      {result && (
        <p
          className="rounded-md bg-muted px-3 py-2 font-mono text-xs data-[status=failed]:text-destructive"
          data-status={result.status}
          role="status"
        >
          {result.status === "succeeded" ? "✓" : "✗"} {result.message}
        </p>
      )}

      {action && (
        <>
          <Separator />
          <div className="grid gap-2">
            <h3 className="text-sm font-medium">Used in</h3>
            <UsageList usages={usages} />
            {usages.length > 0 && (
              <p className="text-xs text-muted-foreground">
                Remove these references before deleting the action.
              </p>
            )}
          </div>
        </>
      )}
    </form>
  )
}
