import { Ban, Copy, Play, Save, Trash2, TriangleAlert } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { DeleteActionDialog, ReferenceList } from "@/components/action-references"
import { IconPicker } from "@/components/icon-picker"
import { ErrorNotice } from "@/components/page"
import { ParameterField } from "@/components/parameter-field"
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
import { Trans, useTranslation } from "@/i18n"
import { api } from "@/lib/api"
import { failureMessage, toastFailure } from "@/lib/failure"
import { creatableTypes } from "@/lib/modules"
import { pluginTypeUnavailable, typeAvailability } from "@/lib/plugin-actions"
import { buildParameters, initialValue, missingRequired } from "@/lib/parameters"
import type {
  ActionInstance,
  ActionReference,
  ActionResult,
  ActionType,
  ModuleInfo,
  ParameterValue,
  Project,
  Workflow,
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
 * module and one of its action types (or with `initialType`, chosen
 * before); the form is then generated from the type's parameter schema,
 * and the backend validates again on save.
 *
 * Deleting a referenced action first replaces or unbinds each reference,
 * as the user decides for each one.
 *
 * Actions whose module is disabled stay visible but cannot be edited or
 * run until the module is enabled again. They can still be deleted.
 */
export function ActionEditor({
  profileId,
  modules,
  projects,
  action,
  initialType,
  references,
  candidates = [],
  workflows,
  canRun,
  onSaved,
  onDeleted,
  onChangeType,
  onOpenReference,
}: {
  profileId: string
  modules: ModuleInfo[]
  /** Projects of the profile, for project parameters. */
  projects: Project[]
  action: ActionInstance | null
  /** For a new action: the type chosen before. */
  initialType?: string
  /** Every place that uses the action. */
  references: ActionReference[]
  /** Actions a reference can use instead when this one is deleted. */
  candidates?: ActionInstance[]
  /** The profile's workflows, for what a Random Action picks from. */
  workflows?: Workflow[]
  canRun: boolean
  onSaved: (action: ActionInstance) => void
  onDeleted: () => void
  /** For a new action with `initialType`: go back to choosing the type. */
  onChangeType?: () => void
  onOpenReference?: (reference: ActionReference) => void
}) {
  const { t } = useTranslation()
  // Types a new action can have: not deprecated, and (plugin actions)
  // available now.
  const offered = (m: ModuleInfo | undefined) =>
    creatableTypes(m?.actionTypes ?? []).filter((type) => !m || !pluginTypeUnavailable(m, type.id))
  const projectsEnabled = modules.some((m) => m.id === "projects" && m.enabled)
  const usable = modules.filter((m) => m.enabled && m.supported && offered(m).length > 0)
  const chosenType = action?.actionType ?? initialType
  const availability = chosenType ? typeAvailability(modules, chosenType) : undefined
  const owner = availability?.owner
  // A legacy type a plugin replaces (module migration): marked deprecated.
  const legacy = chosenType ? owner?.legacy?.[chosenType] : undefined
  // A plugin action that cannot run now cannot be saved, new or not.
  const unavailable = (Boolean(action) || Boolean(availability?.reason)) && !availability?.available
  const [moduleId, setModuleId] = useState(owner?.id ?? (usable.length === 1 ? usable[0]?.id : ""))
  const module = modules.find((m) => m.id === moduleId)
  // Every type, so a saved action of a deprecated type still finds its own.
  const types: ActionType[] = module?.actionTypes ?? []
  // A module with a single action type needs no second choice.
  const onlyType = (m: ModuleInfo | undefined) => {
    const types = offered(m)
    return types.length === 1 ? (types[0]?.id ?? "") : ""
  }
  const [typeId, setTypeId] = useState(chosenType ?? onlyType(module))
  const type = types.find((candidate) => candidate.id === typeId)
  const [name, setName] = useState(action?.name ?? "")
  const [description, setDescription] = useState(action?.description ?? "")
  const [icon, setIcon] = useState<string | null>(action?.icon ?? null)
  const [label, setLabel] = useState(action?.label ?? "")
  const [values, setValues] = useState(() => valuesFor(type, action))
  const [submitted, setSubmitted] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<ActionResult | null>(null)
  const [deleting, setDeleting] = useState(false)

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
      toast.success(action ? t("actions:editor.toast.saved") : t("actions:editor.toast.created"))
      onSaved(saved)
    } catch (e) {
      setError(
        failureMessage(
          action ? t("actions:editor.failure.notSaved") : t("actions:editor.failure.notCreated"),
          e,
        ),
      )
    }
  }

  const duplicate = async () => {
    if (!action) return
    try {
      const copy = await api.duplicateAction(
        profileId,
        action.id,
        t("actions:editor.copyName", { name: action.name }),
      )
      toast.success(t("actions:editor.toast.duplicated"))
      onSaved(copy)
    } catch (e) {
      toastFailure(t("actions:editor.failure.notDuplicated", { name: action.name }), e)
    }
  }

  const run = async () => {
    if (!action) return
    try {
      setResult(await api.runAction(profileId, action.id))
    } catch (e) {
      toastFailure(
        t("actions:editor.failure.didNotRun", { name: action.name }),
        e,
        t("actions:editor.failure.checkAgent"),
      )
    }
  }

  return (
    <form className="grid gap-4" onSubmit={(e) => void save(e)} noValidate>
      {error && <ErrorNotice error={error} />}
      {unavailable && (
        <Alert>
          <Ban />
          <AlertTitle>{t("actions:editor.unavailable.title")}</AlertTitle>
          <AlertDescription>
            {availability?.reason
              ? action
                ? t("actions:editor.unavailable.pluginKept", { reason: availability.reason })
                : t("actions:editor.unavailable.pluginNew", { reason: availability.reason })
              : owner
                ? t("actions:editor.unavailable.moduleDisabled", { module: owner.name })
                : t("actions:editor.unavailable.noModule", { type: action?.actionType ?? "" })}
          </AlertDescription>
        </Alert>
      )}
      {chosenType ? (
        <div className="flex items-start justify-between gap-2">
          <p className="text-sm text-muted-foreground">
            {owner?.name ?? t("actions:editor.unknownModule")} › {type?.name ?? chosenType}
            {type && <span className="block text-xs">{type.description}</span>}
            {legacy && (
              <span className="block text-xs" data-testid="legacy-alias">
                <Trans
                  i18nKey="actions:editor.legacy"
                  values={{
                    module: owner?.name ?? t("actions:editor.theModule"),
                    plugin: legacy.pluginName,
                    target: legacy.target,
                  }}
                  components={{ target: <code /> }}
                />
              </span>
            )}
          </p>
          {!action && onChangeType && (
            <Button type="button" size="sm" variant="outline" onClick={onChangeType}>
              {t("actions:editor.changeType")}
            </Button>
          )}
        </div>
      ) : (
        <div className="grid items-start gap-4 sm:grid-cols-2">
          <div className="grid gap-2">
            <Label htmlFor="action-module">{t("actions:editor.module")}</Label>
            <Select
              value={moduleId}
              onValueChange={(id) => {
                const chosen = modules.find((m) => m.id === id)
                const next = onlyType(chosen)
                setModuleId(id)
                setTypeId(next)
                setValues(
                  valuesFor(
                    chosen?.actionTypes.find((candidate) => candidate.id === next),
                    null,
                  ),
                )
              }}
            >
              <SelectTrigger id="action-module" className="w-full">
                <SelectValue placeholder={t("actions:editor.chooseModule")} />
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
            <Label htmlFor="action-type">{t("actions:editor.actionType")}</Label>
            <Select
              value={typeId}
              disabled={!module}
              onValueChange={(id) => {
                setTypeId(id)
                setValues(
                  valuesFor(
                    types.find((candidate) => candidate.id === id),
                    null,
                  ),
                )
              }}
            >
              <SelectTrigger id="action-type" className="w-full">
                <SelectValue
                  placeholder={
                    module ? t("actions:editor.chooseType") : t("actions:editor.chooseModuleFirst")
                  }
                />
              </SelectTrigger>
              <SelectContent>
                {offered(module).map((candidate) => (
                  <SelectItem key={candidate.id} value={candidate.id}>
                    {candidate.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {type && <p className="text-xs text-muted-foreground">{type.description}</p>}
            {submitted && !type && (
              <p className="text-xs text-destructive">{t("actions:editor.typeRequired")}</p>
            )}
          </div>
        </div>
      )}

      <div className="grid gap-4 sm:grid-cols-2">
        <div className="grid gap-2">
          <Label htmlFor="action-name">{t("actions:editor.name")}</Label>
          <Input
            id="action-name"
            value={name}
            aria-invalid={submitted && nameMissing}
            onChange={(e) => {
              setName(e.target.value)
            }}
            placeholder={t("actions:editor.namePlaceholder")}
          />
          {submitted && nameMissing && (
            <p className="text-xs text-destructive">{t("actions:editor.nameRequired")}</p>
          )}
        </div>
        <div className="grid gap-2">
          <Label htmlFor="action-icon">{t("actions:editor.icon")}</Label>
          <IconPicker id="action-icon" value={icon} onChange={setIcon} />
        </div>
      </div>

      <div className="grid gap-2">
        <Label htmlFor="action-label">{t("actions:editor.keyLabel")}</Label>
        <Input
          id="action-label"
          value={label}
          maxLength={32}
          onChange={(e) => {
            setLabel(e.target.value)
          }}
          placeholder={t("actions:editor.keyLabelPlaceholder")}
        />
        {/* Project placeholders come with the Projects module. */}
        {projectsEnabled && (
          <p className="text-xs text-muted-foreground">
            {t("actions:editor.projectPlaceholders", {
              key: "{project.key}",
              name: "{project.name}",
            })}
          </p>
        )}
      </div>

      <div className="grid gap-2">
        <Label htmlFor="action-description">{t("actions:editor.description")}</Label>
        <Textarea
          id="action-description"
          value={description}
          rows={2}
          onChange={(e) => {
            setDescription(e.target.value)
          }}
          placeholder={t("actions:editor.optional")}
        />
      </div>

      {type?.node?.effect === "modifies" && (
        <Alert>
          <TriangleAlert />
          <AlertTitle>{t("actions:editor.modifies.title")}</AlertTitle>
          <AlertDescription>{t("actions:editor.modifies.body")}</AlertDescription>
        </Alert>
      )}
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
                  actionType={type.id}
                  targets={workflows ? { actions: candidates, workflows } : undefined}
                  onChange={(next) => {
                    setValues((current) => ({ ...current, [spec.key]: next }))
                  }}
                />
                {submitted && missing.includes(spec.key) && (
                  <p className="text-xs text-destructive">
                    {t("actions:editor.parameterRequired", { label: spec.label })}
                  </p>
                )}
              </div>
            ))}
          </div>
        </>
      )}

      <div className="flex flex-wrap gap-2">
        <Button type="submit" disabled={unavailable}>
          <Save /> {action ? t("actions:editor.save") : t("actions:editor.create")}
        </Button>
        {action && (
          <>
            <Button
              type="button"
              variant="outline"
              disabled={unavailable}
              onClick={() => void duplicate()}
            >
              <Copy /> {t("actions:editor.duplicate")}
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={!canRun || unavailable}
              title={canRun ? undefined : t("actions:editor.runHint")}
              onClick={() => void run()}
            >
              <Play /> {t("actions:editor.run")}
            </Button>
            <Button
              type="button"
              variant="ghost"
              onClick={() => {
                setDeleting(true)
              }}
            >
              <Trash2 /> {t("common:actions.delete")}
            </Button>
            {deleting && (
              <DeleteActionDialog
                profileId={profileId}
                action={action}
                references={references}
                candidates={candidates}
                onOpen={onOpenReference}
                onClose={() => {
                  setDeleting(false)
                }}
                onDeleted={() => {
                  setDeleting(false)
                  toast.success(t("actions:editor.toast.deleted"))
                  onDeleted()
                }}
              />
            )}
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
          {result.data && Object.keys(result.data).length > 0 && (
            <span className="mt-1 grid grid-cols-[auto_1fr] gap-x-3 text-muted-foreground">
              {Object.entries(result.data).map(([key, value]) => (
                <span key={key} className="contents">
                  <span>{key}</span>
                  <span className="break-all text-foreground">{value}</span>
                </span>
              ))}
            </span>
          )}
        </p>
      )}

      {action && (
        <>
          <Separator />
          <div className="grid gap-2">
            <h3 className="text-sm font-medium">{t("actions:editor.usedIn")}</h3>
            <ReferenceList references={references} onOpen={onOpenReference} />
            {references.length > 0 && (
              <p className="text-xs text-muted-foreground">{t("actions:editor.deleteHint")}</p>
            )}
          </div>
        </>
      )}
    </form>
  )
}
