import { Lock, Plus, Save, Search, Trash2 } from "lucide-react"
import { useMemo, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState, ErrorNotice, Loading } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
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
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import {
  RESOURCE_KINDS,
  STORED_SCOPES,
  SYSTEM_VARIABLES,
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
  typeLabel,
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
 * Shared variables: named, typed values that actions and workflows read
 * with `{{ shared.<key> }}`, set globally, per profile or per project.
 * System variables are listed read-only. Sensitive values are only ever
 * written here, never shown; a stored-credential variable holds the name
 * of a credential, never its secret.
 */
export function SharedVariablesSettings() {
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
  const [query, setQuery] = useState("")
  const reload = () => {
    setGeneration((g) => g + 1)
  }

  if (data.state.status === "loading") return <Loading />
  if (data.state.status === "error") return <ErrorNotice error={data.state.error} />
  const { variables, projects, credentials } = data.state.data
  const needle = query.trim().toLowerCase()
  const shown = variables.filter(({ definition: d }) =>
    `shared.${d.key} ${d.label ?? ""} ${d.description ?? ""} ${typeLabel(d.valueType)}`
      .toLowerCase()
      .includes(needle),
  )
  const current =
    selected && selected !== NEW ? variables.find((v) => v.definition.key === selected) : undefined

  return (
    <>
      <p className="text-sm text-muted-foreground">
        Values that actions and workflows read with {"{{ shared.<key> }}"}. A value set for a
        project wins over the profile&apos;s, which wins over the global one.
      </p>
      <Card>
        <CardHeader className="gap-3">
          <div className="flex items-center justify-between gap-2">
            <CardTitle>Shared variables</CardTitle>
            <Button
              type="button"
              size="sm"
              onClick={() => {
                setSelected(NEW)
              }}
            >
              <Plus /> New variable
            </Button>
          </div>
          <div className="relative">
            <Search className="absolute top-2.5 left-2 size-4 text-muted-foreground" />
            <Input
              className="pl-8"
              placeholder="Search variables"
              aria-label="Search variables"
              value={query}
              onChange={(e) => {
                setQuery(e.target.value)
              }}
            />
          </div>
        </CardHeader>
        <CardContent>
          {variables.length === 0 ? (
            <EmptyState title="No shared variables yet">
              Create one to use the same value in several actions and workflows.
            </EmptyState>
          ) : shown.length === 0 ? (
            <p className="text-sm text-muted-foreground">No variables match.</p>
          ) : (
            <ul className="grid gap-1" aria-label="Shared variables">
              {shown.map(({ definition: d, references }) => (
                <li key={d.key}>
                  <button
                    type="button"
                    className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none data-[selected=true]:bg-accent"
                    data-selected={selected === d.key}
                    onClick={() => {
                      setSelected(d.key)
                    }}
                  >
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-mono">shared.{d.key}</span>
                      <span className="block truncate text-xs text-muted-foreground">
                        {d.label ? `${d.label} · ` : ""}
                        {typeLabel(d.valueType)}
                      </span>
                    </span>
                    {d.sensitive && (
                      <Badge variant="outline">
                        <Lock /> Sensitive
                      </Badge>
                    )}
                    {references.filter(isDirectUse).length > 0 && (
                      <Badge
                        variant="secondary"
                        title={`Used ${references.filter(isDirectUse).length} time(s)`}
                      >
                        {references.filter(isDirectUse).length}
                      </Badge>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

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

      <SystemVariables />
    </>
  )
}

/**
 * Read-only references DeckPilot provides. The list is the known set; the
 * variable picker shows their current values from the agent.
 */
function SystemVariables() {
  const entries = SYSTEM_VARIABLES
  return (
    <Card>
      <CardHeader>
        <CardTitle>System variables</CardTitle>
        <CardDescription>Read-only values DeckPilot provides when an action runs.</CardDescription>
      </CardHeader>
      <CardContent>
        <ul className="grid gap-2 text-sm" aria-label="System variables">
          {entries.map((entry) => (
            <li key={entry.reference} className="grid">
              <span className="font-mono">{`{{ ${entry.reference} }}`}</span>
              <span className="text-xs text-muted-foreground">
                {entry.label} · {typeLabel(entry.valueType)}
                {entry.availability === "runtimeOnly" ? " · only while a workflow runs" : ""}
                {entry.description ? ` — ${entry.description}` : ""}
              </span>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
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
        "Choose a project",
      )
    case "profileId":
      return choose(
        profiles.map((p) => ({ value: p.id, label: p.name })),
        "Choose a profile",
      )
    case "secretRef":
      return credentials.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No stored credentials. Add one under Security first; the secret stays there.
        </p>
      ) : (
        choose(
          credentials.map((c) => ({
            value: c.id,
            label: c.description ? `${c.id} — ${c.description}` : c.id,
          })),
          "Choose a stored credential",
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
            <SelectTrigger className="w-full" aria-label={`${label}: kind of reference`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="fixed">Resource key</SelectItem>
              <SelectItem value="projectRole">Project role</SelectItem>
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
  const scopeError = scopes.length === 0 ? "Choose at least one scope." : null
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
      toast.success(saved ? "Variable saved" : "Variable created")
      onSaved(definition.key)
    } catch (e) {
      setError(errorMessage(e))
    }
  }

  const remove = async () => {
    if (!saved) return
    try {
      await api.deleteSharedVariable(saved.key)
      toast.success("Variable deleted")
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
    <Card>
      <CardHeader>
        <CardTitle>{saved ? `shared.${saved.key}` : "New variable"}</CardTitle>
        {saved?.module && <CardDescription>Declared by the {saved.module} module</CardDescription>}
      </CardHeader>
      <CardContent className="grid gap-6">
        <form className="grid gap-4" onSubmit={(e) => void save(e)} noValidate>
          {error && <ErrorNotice error={error} />}
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-2">
              <Label htmlFor="variable-key">Key</Label>
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
                  Referenced as {`{{ shared.${(saved?.key ?? key) || "<key>"} }}`}
                  {saved ? "; the key cannot change." : ""}
                </p>
              )}
            </div>
            <div className="grid gap-2">
              <Label htmlFor="variable-label">Label</Label>
              <Input
                id="variable-label"
                value={label}
                placeholder="Optional"
                onChange={(e) => {
                  setLabel(e.target.value)
                }}
              />
            </div>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="variable-description">Description</Label>
            <Textarea
              id="variable-description"
              rows={2}
              value={description}
              placeholder="Optional"
              onChange={(e) => {
                setDescription(e.target.value)
              }}
            />
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-2">
              <Label htmlFor="variable-type">Type</Label>
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
                  {VALUE_TYPES.map((t) => (
                    <SelectItem key={t.type} value={t.type}>
                      {t.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {valueType.type === "secretRef" && (
                <p className="text-xs text-muted-foreground">
                  Holds the name of a stored credential. The secret stays in the credential store
                  and is never shown here.
                </p>
              )}
            </div>
            {valueType.type === "resource" && (
              <div className="grid gap-2">
                <Label htmlFor="variable-resource-kind">Resource kind</Label>
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
                    <SelectItem value="any">Any kind</SelectItem>
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
            <legend className="text-sm font-medium">Scopes</legend>
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
              <Label htmlFor="variable-sensitive">Sensitive</Label>
              <span className="text-xs text-muted-foreground">
                Values are never shown again, in logs or in history.
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
                Default value, used where no scope has one
              </label>
              {hasDefault && (
                <ValueInput
                  id="variable-default"
                  label="Default value"
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
              <Save /> {saved ? "Save" : "Create variable"}
            </Button>
            {saved && (
              <ConfirmDialog
                trigger={
                  <Button type="button" variant="ghost" disabled={uses.length > 0}>
                    <Trash2 /> Delete
                  </Button>
                }
                title={`Delete “shared.${saved.key}”?`}
                description="The variable and all its values are removed. This cannot be undone."
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
              <h3 className="text-sm font-medium">Used in</h3>
              {references.length === 0 ? (
                <p className="text-sm text-muted-foreground">Not used by any action or workflow.</p>
              ) : (
                <>
                  <ul className="grid gap-1 text-sm text-muted-foreground" aria-label="Usages">
                    {references.map((reference, index) => (
                      <li key={index}>{describeVariableReference(reference)}</li>
                    ))}
                  </ul>
                  <p className="text-xs text-muted-foreground">
                    Remove these references before deleting the variable.
                  </p>
                </>
              )}
            </div>
          </>
        )}
      </CardContent>
    </Card>
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
      toast.success("Value saved")
      onChanged()
    } catch (e) {
      setProblem(errorMessage(e))
    }
  }

  const clear = async (scope: ScopeKey) => {
    try {
      await api.clearSharedVariableValue(definition.key, scope)
      toast.success("Value cleared")
      onChanged()
    } catch (e) {
      toast.error(errorMessage(e))
    }
  }

  return (
    <div className="grid gap-2">
      <h3 className="text-sm font-medium">Values</h3>
      {slots.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {profileId
            ? "No scope allows a value here."
            : "Choose a profile to set profile and project values."}
        </p>
      ) : (
        <ul className="grid gap-3" aria-label="Values">
          {slots.map((scope) => {
            const entry = valueAt(scope)
            const name = scopeLabel(scope, projects, profiles)
            const shown = entry
              ? sensitive
                ? "Set (redacted)"
                : formatValue(definition, entry.value, projects, profiles)
              : "Not set"
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
                        aria-label={`${entry ? "Change" : "Set"} ${name}`}
                        onClick={() => {
                          start(scope)
                        }}
                      >
                        {entry ? "Change" : "Set"}
                      </Button>
                      {entry && (
                        <Button
                          type="button"
                          size="sm"
                          variant="ghost"
                          aria-label={`Clear ${name}`}
                          onClick={() => void clear(scope)}
                        >
                          Clear
                        </Button>
                      )}
                    </span>
                  )}
                </div>
                {isEditing && (
                  <div className="grid gap-2">
                    <ValueInput
                      id="variable-value"
                      label={`Value for ${name}`}
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
                        Save value
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        onClick={() => {
                          setEditing(null)
                        }}
                      >
                        Cancel
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
