import { Plus, Save, Trash2 } from "lucide-react"
import { useMemo, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState, ErrorNotice, Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Panel, SettingsSection } from "@/components/settings-layout"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { useAsync } from "@/hooks/use-async"
import { useTranslation } from "@/i18n"
import { VariablesTable } from "@/pages/variables-table"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import {
  RESOURCE_KINDS,
  STORED_SCOPES,
  VALUE_TYPES,
  type ValueDraft,
  describeVariableReference,
  draftOf,
  emptyDraft,
  formatValue,
  isDirectUse,
  keyProblem,
  parseValue,
  sameScope,
  scopeLabel,
  slotsFor,
} from "@/lib/shared-variables"
import type {
  CredentialInfo,
  ProfileSummary,
  Project,
  ResourceKind,
  ScopeKey,
  SharedVariable,
  VariableDefinition,
  VariableScope,
  VariableType,
} from "@/lib/types"

interface Data {
  variables: SharedVariable[]
  projects: Project[]
  credentials: CredentialInfo[]
}

const NEW = "__new__"

/**
 * Variables: one table of every variable, whoever owns it (DeckPilot, a
 * module or the user), with search and owner, type and scope filters
 * (docs/variables.md). Core and module variables are read-only; shared
 * variables — named, typed values that actions and workflows read with
 * `{{ shared.<key> }}`, set globally, per profile or per project — are
 * created, edited and deleted here. Sensitive values are only ever
 * written here, never shown; a stored-credential variable holds the name
 * of a credential, never its secret.
 */
export function SharedVariablesSettings() {
  const { t } = useTranslation()
  const { editingProfileId, profiles, revision } = useAppContext()
  const profileId = editingProfileId
  const [generation, setGeneration] = useState(0)
  const data = useAsync<Data>(
    async () => {
      const [variables, view, credentials] = await Promise.all([
        api.listSharedVariables(profileId),
        profileId ? api.getProfile(profileId) : Promise.resolve(null),
        api.listCredentials(),
      ])
      return { variables, projects: view?.profile.projects ?? [], credentials }
    },
    `${profileId ?? ""}:${revision}:${generation}`,
  )
  const [selected, setSelected] = useState<string | null>(null)
  const reload = () => {
    setGeneration((g) => g + 1)
  }

  if (data.state.status === "loading") return <Loading />
  if (data.state.status === "error") return <ErrorNotice error={data.state.error} />
  const { variables, projects, credentials } = data.state.data
  const current =
    selected && selected !== NEW ? variables.find((v) => v.definition.key === selected) : undefined

  return (
    <>
      <p className="text-sm text-muted-foreground">
        {t("workflows:sharedVariables.intro", { syntax: "{{ reference }}" })}
      </p>
      <SettingsSection
        title={t("workflows:sharedVariables.title")}
        actions={
          <Button
            type="button"
            size="sm"
            onClick={() => {
              setSelected(NEW)
            }}
          >
            <Plus /> {t("workflows:sharedVariables.new")}
          </Button>
        }
      >
        {variables.length === 0 && (
          <EmptyState variant="plain" title={t("workflows:sharedVariables.empty.title")}>
            {t("workflows:sharedVariables.empty.description")}
          </EmptyState>
        )}
        <VariablesTable shared={variables} selected={selected} onEdit={setSelected} />
      </SettingsSection>

      {(current ?? selected === NEW) && (
        <VariableEditor
          key={current?.definition.key ?? NEW}
          variable={current ?? null}
          existingKeys={variables.map((v) => v.definition.key)}
          profileId={profileId}
          projects={projects}
          profiles={profiles.profiles}
          credentials={credentials}
          onSaved={(key) => {
            setSelected(key)
            reload()
          }}
          onDeleted={() => {
            setSelected(null)
            reload()
          }}
        />
      )}
    </>
  )
}

function ValueInput({
  id,
  label,
  type,
  sensitive,
  draft,
  projects,
  profiles,
  credentials,
  onChange,
}: {
  id: string
  label: string
  type: VariableType
  sensitive: boolean
  draft: ValueDraft
  projects: Project[]
  profiles: ProfileSummary[]
  credentials: CredentialInfo[]
  onChange: (draft: ValueDraft) => void
}) {
  const { t } = useTranslation()
  const choose = (options: { value: string; label: string }[], placeholder: string) => (
    <Select
      value={draft.text}
      onValueChange={(text) => {
        onChange({ text })
      }}
    >
      <SelectTrigger id={id} className="w-full" aria-label={label}>
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        {options.map((o) => (
          <SelectItem key={o.value} value={o.value}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
  switch (type.type) {
    case "boolean":
      return (
        <Switch
          id={id}
          aria-label={label}
          checked={draft.text === "true"}
          onCheckedChange={(checked) => {
            onChange({ text: checked ? "true" : "false" })
          }}
        />
      )
    case "projectId":
      return choose(
        projects.map((p) => ({ value: p.id, label: `${p.key} — ${p.name}` })),
        t("workflows:sharedVariables.chooseProject"),
      )
    case "profileId":
      return choose(
        profiles.map((p) => ({ value: p.id, label: p.name })),
        t("workflows:sharedVariables.chooseProfile"),
      )
    case "secretRef":
      return credentials.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t("workflows:sharedVariables.noCredentials")}
        </p>
      ) : (
        choose(
          credentials.map((c) => ({
            value: c.id,
            label: c.description ? `${c.id} — ${c.description}` : c.id,
          })),
          t("workflows:sharedVariables.chooseCredential"),
        )
      )
    case "resource":
      return (
        <div className="grid gap-2 sm:grid-cols-[12rem_1fr]">
          <Select
            value={draft.mode ?? "fixed"}
            onValueChange={(mode) => {
              onChange({ ...draft, mode: mode as ValueDraft["mode"] })
            }}
          >
            <SelectTrigger
              className="w-full"
              aria-label={t("workflows:sharedVariables.referenceKind", { label })}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="fixed">{t("workflows:sharedVariables.resourceKey")}</SelectItem>
              <SelectItem value="projectRole">
                {t("workflows:sharedVariables.projectRole")}
              </SelectItem>
            </SelectContent>
          </Select>
          <Input
            id={id}
            aria-label={label}
            value={draft.text}
            onChange={(e) => {
              onChange({ ...draft, text: e.target.value })
            }}
          />
        </div>
      )
    case "json":
      return sensitive ? (
        <Input
          id={id}
          type="password"
          autoComplete="off"
          aria-label={label}
          value={draft.text}
          onChange={(e) => {
            onChange({ text: e.target.value })
          }}
        />
      ) : (
        <Textarea
          id={id}
          aria-label={label}
          rows={3}
          className="font-mono"
          value={draft.text}
          onChange={(e) => {
            onChange({ text: e.target.value })
          }}
        />
      )
    default:
      return (
        <Input
          id={id}
          aria-label={label}
          type={sensitive ? "password" : "text"}
          autoComplete="off"
          inputMode={type.type === "integer" ? "numeric" : undefined}
          value={draft.text}
          onChange={(e) => {
            onChange({ text: e.target.value })
          }}
        />
      )
  }
}

function VariableEditor({
  variable,
  existingKeys,
  profileId,
  projects,
  profiles,
  credentials,
  onSaved,
  onDeleted,
}: {
  variable: SharedVariable | null
  existingKeys: string[]
  profileId: string | null
  projects: Project[]
  profiles: ProfileSummary[]
  credentials: CredentialInfo[]
  onSaved: (key: string) => void
  onDeleted: () => void
}) {
  const { t } = useTranslation()
  const saved = variable?.definition
  const [key, setKey] = useState(saved?.key ?? "")
  const [label, setLabel] = useState(saved?.label ?? "")
  const [description, setDescription] = useState(saved?.description ?? "")
  const [valueType, setValueType] = useState<VariableType>(saved?.valueType ?? { type: "string" })
  const [scopes, setScopes] = useState<VariableScope[]>(saved?.scopes ?? ["global"])
  const [sensitive, setSensitive] = useState(saved?.sensitive ?? false)
  const [hasDefault, setHasDefault] = useState(Boolean(saved?.default) && !saved?.sensitive)
  const [defaultDraft, setDefaultDraft] = useState<ValueDraft>(() =>
    saved?.default && !saved.sensitive ? draftOf(saved.default) : emptyDraft(valueType),
  )
  const [submitted, setSubmitted] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const keyError = saved ? null : keyProblem(key, existingKeys)
  const scopeError = scopes.length === 0 ? t("workflows:sharedVariables.scopeRequired") : null
  const parsedDefault = hasDefault && !sensitive ? parseValue(valueType, defaultDraft) : null
  const defaultError = parsedDefault && "error" in parsedDefault ? parsedDefault.error : null

  const save = async (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    setSubmitted(true)
    if (keyError || scopeError || defaultError) return
    setError(null)
    const definition: VariableDefinition = {
      key: saved?.key ?? key.trim(),
      label: label.trim(),
      description: description.trim(),
      valueType,
      scopes,
      sensitive,
      ...(parsedDefault && "value" in parsedDefault ? { default: parsedDefault.value } : {}),
      ...(saved?.module ? { module: saved.module } : {}),
    }
    try {
      await api.saveSharedVariable(definition)
      toast.success(
        saved
          ? t("workflows:sharedVariables.toast.saved")
          : t("workflows:sharedVariables.toast.created"),
      )
      onSaved(definition.key)
    } catch (e) {
      setError(errorMessage(e))
    }
  }

  const remove = async () => {
    if (!saved) return
    try {
      await api.deleteSharedVariable(saved.key)
      toast.success(t("workflows:sharedVariables.toast.deleted"))
      onDeleted()
    } catch (e) {
      toast.error(errorMessage(e))
    }
  }

  const references = variable?.references ?? []
  const uses = references.filter(isDirectUse)
  const definitionNow: VariableDefinition = {
    key: saved?.key ?? key,
    valueType,
    scopes,
    sensitive,
  }

  return (
    <Panel
      title={saved ? `shared.${saved.key}` : t("workflows:sharedVariables.new")}
      description={
        saved?.module && t("workflows:sharedVariables.declaredBy", { module: saved.module })
      }
    >
      <form className="grid gap-4" onSubmit={(e) => void save(e)} noValidate>
        {error && <ErrorNotice error={error} />}
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="grid gap-2">
            <Label htmlFor="variable-key">{t("workflows:sharedVariables.key")}</Label>
            <Input
              id="variable-key"
              value={key}
              readOnly={Boolean(saved)}
              placeholder="paths.root"
              aria-invalid={submitted && Boolean(keyError)}
              onChange={(e) => {
                setKey(e.target.value)
              }}
            />
            {submitted && keyError ? (
              <p className="text-xs text-destructive">{keyError}</p>
            ) : (
              <p className="text-xs text-muted-foreground">
                {saved
                  ? t("workflows:sharedVariables.referencedAsFixed", {
                      reference: `{{ shared.${saved.key} }}`,
                    })
                  : t("workflows:sharedVariables.referencedAs", {
                      reference: `{{ shared.${key || `<${t("workflows:sharedVariables.keyPlaceholder")}>`} }}`,
                    })}
              </p>
            )}
          </div>
          <div className="grid gap-2">
            <Label htmlFor="variable-label">{t("workflows:sharedVariables.label")}</Label>
            <Input
              id="variable-label"
              value={label}
              placeholder={t("workflows:sharedVariables.optional")}
              onChange={(e) => {
                setLabel(e.target.value)
              }}
            />
          </div>
        </div>
        <div className="grid gap-2">
          <Label htmlFor="variable-description">{t("workflows:sharedVariables.description")}</Label>
          <Textarea
            id="variable-description"
            rows={2}
            value={description}
            placeholder={t("workflows:sharedVariables.optional")}
            onChange={(e) => {
              setDescription(e.target.value)
            }}
          />
        </div>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="grid gap-2">
            <Label htmlFor="variable-type">{t("workflows:sharedVariables.type")}</Label>
            <Select
              value={valueType.type}
              disabled={Boolean(saved)}
              onValueChange={(type) => {
                const next = { type } as VariableType
                setValueType(next)
                setDefaultDraft(emptyDraft(next))
                if (type === "secretRef") setSensitive(false)
              }}
            >
              <SelectTrigger id="variable-type" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {VALUE_TYPES.map((v) => (
                  <SelectItem key={v.type} value={v.type}>
                    {v.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {valueType.type === "secretRef" && (
              <p className="text-xs text-muted-foreground">
                {t("workflows:sharedVariables.secretRefHint")}
              </p>
            )}
          </div>
          {valueType.type === "resource" && (
            <div className="grid gap-2">
              <Label htmlFor="variable-resource-kind">
                {t("workflows:sharedVariables.resourceKind")}
              </Label>
              <Select
                value={valueType.kind ?? "any"}
                disabled={Boolean(saved)}
                onValueChange={(kind) => {
                  setValueType(
                    kind === "any"
                      ? { type: "resource" }
                      : { type: "resource", kind: kind as ResourceKind },
                  )
                }}
              >
                <SelectTrigger id="variable-resource-kind" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="any">{t("workflows:sharedVariables.anyKind")}</SelectItem>
                  {RESOURCE_KINDS.map((k) => (
                    <SelectItem key={k.kind} value={k.kind}>
                      {k.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
        </div>
        <fieldset className="grid gap-2">
          <legend className="text-sm font-medium">{t("workflows:sharedVariables.scopes")}</legend>
          {STORED_SCOPES.map((s) => (
            <label key={s.scope} className="flex items-center gap-2 text-sm">
              <Checkbox
                checked={scopes.includes(s.scope)}
                onCheckedChange={(checked) => {
                  setScopes((current) =>
                    checked === true
                      ? [...current.filter((c) => c !== s.scope), s.scope]
                      : current.filter((c) => c !== s.scope),
                  )
                }}
              />
              {s.label}
              <span className="text-xs text-muted-foreground">{s.hint}</span>
            </label>
          ))}
          {submitted && scopeError && <p className="text-xs text-destructive">{scopeError}</p>}
        </fieldset>
        {valueType.type !== "secretRef" && (
          <div className="flex items-center gap-2">
            <Switch
              id="variable-sensitive"
              checked={sensitive}
              onCheckedChange={(checked) => {
                setSensitive(checked)
                if (checked) setHasDefault(false)
              }}
            />
            <Label htmlFor="variable-sensitive">{t("workflows:sharedVariables.sensitive")}</Label>
            <span className="text-xs text-muted-foreground">
              {t("workflows:sharedVariables.sensitiveHint")}
            </span>
          </div>
        )}
        {!sensitive && (
          <div className="grid gap-2">
            <label className="flex items-center gap-2 text-sm">
              <Checkbox
                checked={hasDefault}
                onCheckedChange={(checked) => {
                  setHasDefault(checked === true)
                }}
              />
              {t("workflows:sharedVariables.defaultToggle")}
            </label>
            {hasDefault && (
              <ValueInput
                id="variable-default"
                label={t("workflows:sharedVariables.defaultValue")}
                type={valueType}
                sensitive={false}
                draft={defaultDraft}
                projects={projects}
                profiles={profiles}
                credentials={credentials}
                onChange={setDefaultDraft}
              />
            )}
            {submitted && defaultError && (
              <p className="text-xs text-destructive">{defaultError}</p>
            )}
          </div>
        )}
        <div className="flex flex-wrap gap-2">
          <Button type="submit">
            <Save />{" "}
            {saved ? t("workflows:sharedVariables.save") : t("workflows:sharedVariables.create")}
          </Button>
          {saved && (
            <ConfirmDialog
              trigger={
                <Button type="button" variant="ghost" disabled={uses.length > 0}>
                  <Trash2 /> {t("common:actions.delete")}
                </Button>
              }
              title={t("workflows:sharedVariables.deleteTitle", {
                reference: `shared.${saved.key}`,
              })}
              description={t("workflows:sharedVariables.deleteDescription")}
              onConfirm={() => void remove()}
            />
          )}
        </div>
      </form>

      {variable && (
        <>
          <Values
            variable={{ ...variable, definition: { ...variable.definition, ...definitionNow } }}
            profileId={profileId}
            projects={projects}
            profiles={profiles}
            credentials={credentials}
            onChanged={() => {
              onSaved(variable.definition.key)
            }}
          />
          <div className="grid gap-2">
            <h4 className="text-sm font-medium">{t("workflows:sharedVariables.usedIn")}</h4>
            {references.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                {t("workflows:sharedVariables.notUsed")}
              </p>
            ) : (
              <>
                <ul
                  className="grid gap-1 text-sm text-muted-foreground"
                  aria-label={t("workflows:sharedVariables.usages")}
                >
                  {references.map((reference, index) => (
                    <li key={index}>{describeVariableReference(reference)}</li>
                  ))}
                </ul>
                <p className="text-xs text-muted-foreground">
                  {t("workflows:sharedVariables.removeReferences")}
                </p>
              </>
            )}
          </div>
        </>
      )}
    </Panel>
  )
}

function Values({
  variable,
  profileId,
  projects,
  profiles,
  credentials,
  onChanged,
}: {
  variable: SharedVariable
  profileId: string | null
  projects: Project[]
  profiles: ProfileSummary[]
  credentials: CredentialInfo[]
  onChanged: () => void
}) {
  const { t } = useTranslation()
  const { definition } = variable
  const slots = useMemo(
    () => slotsFor(definition, profileId, projects),
    [definition, profileId, projects],
  )
  const [editing, setEditing] = useState<ScopeKey | null>(null)
  const [draft, setDraft] = useState<ValueDraft>(emptyDraft(definition.valueType))
  const [problem, setProblem] = useState<string | null>(null)
  const sensitive = Boolean(definition.sensitive)

  const valueAt = (scope: ScopeKey) => variable.values.find((v) => sameScope(v.scope, scope))

  const start = (scope: ScopeKey) => {
    const existing = valueAt(scope)?.value
    // A sensitive value is never put back into a field.
    setDraft(existing && !sensitive ? draftOf(existing) : emptyDraft(definition.valueType))
    setProblem(null)
    setEditing(scope)
  }

  const save = async (scope: ScopeKey) => {
    const parsed = parseValue(definition.valueType, draft)
    if ("error" in parsed) {
      setProblem(parsed.error)
      return
    }
    try {
      await api.setSharedVariableValue(definition.key, scope, parsed.value)
      setEditing(null)
      toast.success(t("workflows:sharedVariables.toast.valueSaved"))
      onChanged()
    } catch (e) {
      setProblem(errorMessage(e))
    }
  }

  const clear = async (scope: ScopeKey) => {
    try {
      await api.clearSharedVariableValue(definition.key, scope)
      toast.success(t("workflows:sharedVariables.toast.valueCleared"))
      onChanged()
    } catch (e) {
      toast.error(errorMessage(e))
    }
  }

  return (
    <div className="grid gap-2">
      <h4 className="text-sm font-medium">{t("workflows:sharedVariables.values")}</h4>
      {slots.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {profileId
            ? t("workflows:sharedVariables.noSlots")
            : t("workflows:sharedVariables.noProfile")}
        </p>
      ) : (
        <ul className="grid gap-3" aria-label={t("workflows:sharedVariables.values")}>
          {slots.map((scope) => {
            const entry = valueAt(scope)
            const name = scopeLabel(scope, projects, profiles)
            const shown = entry
              ? sensitive
                ? t("workflows:sharedVariables.redacted")
                : formatValue(definition, entry.value, projects, profiles)
              : t("workflows:sharedVariables.notSet")
            const isEditing = editing !== null && sameScope(editing, scope)
            return (
              <li key={JSON.stringify(scope)} className="grid gap-2" aria-label={name}>
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="text-sm">
                    <span className="font-medium">{name}</span>{" "}
                    <span className="text-muted-foreground">{shown}</span>
                  </span>
                  {!isEditing && (
                    <span className="flex gap-1">
                      <Button
                        type="button"
                        size="sm"
                        variant="outline"
                        aria-label={
                          entry
                            ? t("workflows:sharedVariables.changeNamed", { name })
                            : t("workflows:sharedVariables.setNamed", { name })
                        }
                        onClick={() => {
                          start(scope)
                        }}
                      >
                        {entry
                          ? t("workflows:sharedVariables.change")
                          : t("workflows:sharedVariables.set")}
                      </Button>
                      {entry && (
                        <Button
                          type="button"
                          size="sm"
                          variant="ghost"
                          aria-label={t("workflows:sharedVariables.clearNamed", { name })}
                          onClick={() => void clear(scope)}
                        >
                          {t("common:actions.clear")}
                        </Button>
                      )}
                    </span>
                  )}
                </div>
                {isEditing && (
                  <div className="grid gap-2">
                    <ValueInput
                      id="variable-value"
                      label={t("workflows:sharedVariables.valueFor", { name })}
                      type={definition.valueType}
                      sensitive={sensitive}
                      draft={draft}
                      projects={projects}
                      profiles={profiles}
                      credentials={credentials}
                      onChange={setDraft}
                    />
                    {problem && <p className="text-xs text-destructive">{problem}</p>}
                    <span className="flex gap-2">
                      <Button type="button" size="sm" onClick={() => void save(scope)}>
                        {t("workflows:sharedVariables.saveValue")}
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        onClick={() => {
                          setEditing(null)
                        }}
                      >
                        {t("common:actions.cancel")}
                      </Button>
                    </span>
                  </div>
                )}
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}
