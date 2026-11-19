// Configure: a plugin's settings and credentials, against the agent's
// `getPluginSettings` / `setPluginSettings` and `listPluginCredentials` /
// `setPluginCredential` / `deletePluginCredential`.
//
// Credentials: the agent never returns a secret, only whether one is
// stored. The secret the user types lives in an uncontrolled password
// input (the DOM only, never React state, props or context); on submit it
// is read once, the input is cleared at once, and it goes to the agent.
// Nothing here logs it, puts it into browser storage, or shows it again.

import { RotateCcw } from "lucide-react"
import { useEffect, useId, useRef, useState, type SyntheticEvent } from "react"
import { toast } from "sonner"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"

import { pluginErrorText, type PluginClient, type PluginErrorText } from "./api"
import { Notice } from "./components"
import { formatTime, secretProblem } from "./model"
import {
  defaultText,
  draftOf,
  missingLabels,
  settingControl,
  settingsChanges,
  settingsProblems,
  type SettingDraft,
} from "./settings-form"
import type {
  PluginCredentialSlot,
  PluginDetails,
  PluginSettingField,
  PluginSettings,
} from "./types"
import { UserOriginsSection } from "./user-origins"

type Loaded<T> = { status: "loading" } | { status: "error"; error: PluginErrorText } | T

export function ConfigureDialog({
  item,
  client,
  onClose,
  onChanged,
}: {
  item: PluginDetails
  client: PluginClient
  onClose: () => void
  /** Settings or credentials changed (the list reads the plugin again). */
  onChanged: () => void
}) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent className="max-h-[90svh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Configure {item.summary.name}</DialogTitle>
          <DialogDescription>
            Settings and credentials of version {item.summary.version}, stored by the agent for this
            plugin only.
          </DialogDescription>
        </DialogHeader>
        <PluginConfiguration
          id={item.summary.id}
          name={item.summary.name}
          client={client}
          onClose={onClose}
          onChanged={onChanged}
        />
      </DialogContent>
    </Dialog>
  )
}

/**
 * A plugin's settings, server origins and credentials: in the configure
 * dialog and in the plugin's settings view of the toolbar (a view the app
 * draws; the plugin provides only its declared fields).
 */
export function PluginConfiguration({
  id,
  name,
  client,
  onClose,
  onChanged,
}: {
  id: string
  name: string
  client: PluginClient
  /** "Cancel" of the settings form. */
  onClose: () => void
  /** Settings or credentials changed. */
  onChanged: () => void
}) {
  const [settings, setSettings] = useState<Loaded<{ status: "ready"; settings: PluginSettings }>>({
    status: "loading",
  })
  const [slots, setSlots] = useState<
    Loaded<{ status: "ready"; credentials: PluginCredentialSlot[] }>
  >({ status: "loading" })

  useEffect(() => {
    let active = true
    client.getSettings(id).then(
      (value) => {
        if (active) setSettings({ status: "ready", settings: value })
      },
      (error: unknown) => {
        if (active) setSettings({ status: "error", error: pluginErrorText(error) })
      },
    )
    client.listCredentials(id).then(
      (value) => {
        if (active) setSlots({ status: "ready", credentials: value })
      },
      (error: unknown) => {
        if (active) setSlots({ status: "error", error: pluginErrorText(error) })
      },
    )
    return () => {
      active = false
    }
  }, [client, id])

  const hasSettings = settings.status !== "ready" || settings.settings.fields.length > 0
  const hasCredentials = slots.status !== "ready" || slots.credentials.length > 0

  return (
    <>
      {hasSettings && (
        <section className="grid gap-3" aria-labelledby="configure-settings">
          <h4
            id="configure-settings"
            className="text-xs font-medium text-muted-foreground uppercase"
          >
            Settings
          </h4>
          {settings.status === "loading" ? (
            <p className="text-sm text-muted-foreground">Loading settings…</p>
          ) : settings.status === "error" ? (
            <ErrorText error={settings.error} />
          ) : (
            <SettingsForm
              key={JSON.stringify(settings.settings.values)}
              settings={settings.settings}
              onCancel={onClose}
              save={async (values) => {
                const next = await client.setSettings(id, values)
                setSettings({ status: "ready", settings: next })
                onChanged()
                return next
              }}
            />
          )}
        </section>
      )}
      {settings.status === "ready" && (settings.settings.origins ?? []).length > 0 && (
        <UserOriginsSection
          plugin={name}
          origins={settings.settings.origins ?? []}
          grant={(setting, origin, privateNetwork) =>
            client.grantUserOrigin(id, setting, origin, privateNetwork)
          }
          revoke={(setting) => client.revokeUserOrigin(id, setting)}
          onSettings={(next) => {
            setSettings({ status: "ready", settings: next })
            onChanged()
          }}
        />
      )}
      {hasCredentials && (
        <section className="grid gap-3" aria-labelledby="configure-credentials">
          <h4
            id="configure-credentials"
            className="text-xs font-medium text-muted-foreground uppercase"
          >
            Credentials
          </h4>
          {slots.status === "loading" ? (
            <p className="text-sm text-muted-foreground">Loading credentials…</p>
          ) : slots.status === "error" ? (
            <ErrorText error={slots.error} />
          ) : (
            <>
              <p className="text-xs text-muted-foreground">
                A secret goes to the agent once and is never shown again, here or anywhere. The
                plugin never sees it: the agent adds it only to requests to the hosts listed.
              </p>
              <ul className="grid divide-y border-y" aria-label={`Credentials of ${name}`}>
                {slots.credentials.map((slot) => (
                  <CredentialRow
                    key={slot.key}
                    plugin={name}
                    slot={slot}
                    set={(secret) => client.setCredential(id, slot.key, secret)}
                    remove={() => client.deleteCredential(id, slot.key)}
                    onSlots={(next) => {
                      setSlots({ status: "ready", credentials: next })
                      onChanged()
                    }}
                  />
                ))}
              </ul>
            </>
          )}
        </section>
      )}
      {!hasSettings && !hasCredentials && (
        <p className="text-sm text-muted-foreground">
          This version declares no settings and no credentials.
        </p>
      )}
    </>
  )
}

function ErrorText({ error }: { error: PluginErrorText }) {
  return (
    <p role="alert" className="text-sm text-destructive">
      {error.title}: {error.message}
      {error.hint ? ` ${error.hint}` : ""}
    </p>
  )
}

const DEFAULT_CHOICE = "\u0000default"

function SettingsForm({
  settings,
  save,
  onCancel,
}: {
  settings: PluginSettings
  save: (values: Record<string, string | number | boolean | null>) => Promise<PluginSettings>
  onCancel: () => void
}) {
  const [draft, setDraft] = useState<SettingDraft>(() => draftOf(settings))
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [formErrors, setFormErrors] = useState<string[]>([])
  const [saving, setSaving] = useState(false)
  const missing = missingLabels(settings)
  const pending = settingsChanges(settings, draft)
  // Something to save, or something to explain on submit.
  const dirty = Object.keys(pending.values).length > 0 || Object.keys(pending.errors).length > 0

  const submit = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    const { values, errors: local } = settingsChanges(settings, draft)
    setErrors(local)
    setFormErrors([])
    if (Object.keys(local).length > 0 || Object.keys(values).length === 0) return
    setSaving(true)
    save(values)
      .then(() => {
        toast.success("Settings saved")
      })
      .catch((error: unknown) => {
        const text = pluginErrorText(error)
        if (text.reason === "invalidData") {
          const problems = settingsProblems(text.message, settings.fields)
          setErrors(problems.fields)
          setFormErrors(problems.form)
        } else {
          setFormErrors([`${text.title}: ${text.message}${text.hint ? ` ${text.hint}` : ""}`])
        }
      })
      .finally(() => {
        setSaving(false)
      })
  }

  return (
    <form className="grid gap-4" aria-label="Settings" noValidate onSubmit={submit}>
      {missing.length > 0 && (
        <Notice title="Required settings are missing">
          <p>
            {missing.join(", ")} {missing.length === 1 ? "has" : "have"} no value and no default.
            The plugin runs without {missing.length === 1 ? "it" : "them"}, and its actions may fail
            until {missing.length === 1 ? "it is" : "they are"} set.
          </p>
        </Notice>
      )}
      {settings.fields.map((field) => (
        <SettingRow
          key={field.key}
          field={field}
          value={draft[field.key] ?? null}
          saved={settings.values[field.key] !== undefined}
          missing={(settings.missing ?? []).includes(field.key)}
          error={errors[field.key]}
          onChange={(value) => {
            setDraft({ ...draft, [field.key]: value })
            if (errors[field.key]) {
              setErrors(Object.fromEntries(Object.entries(errors).filter(([k]) => k !== field.key)))
            }
          }}
        />
      ))}
      {formErrors.length > 0 && (
        <div role="alert" className="grid gap-1 text-sm text-destructive">
          {formErrors.map((e) => (
            <p key={e}>{e}</p>
          ))}
        </div>
      )}
      <p className="text-xs text-muted-foreground">
        Settings are not secret. Never enter a password or token here; the plugin's credentials
        below are the place for secrets.
      </p>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" disabled={saving || !dirty}>
          {saving ? "Saving…" : "Save settings"}
        </Button>
      </DialogFooter>
    </form>
  )
}

function SettingRow({
  field,
  value,
  saved,
  missing,
  error,
  onChange,
}: {
  field: PluginSettingField
  value: string | boolean | null
  /** The agent has a saved value (the default does not apply). */
  saved: boolean
  missing: boolean
  error: string | undefined
  onChange: (value: string | boolean | null) => void
}) {
  const id = useId()
  const control = settingControl(field)
  const fallback = defaultText(field)
  const describedBy = [`${id}-help`, error ? `${id}-error` : null].filter(Boolean).join(" ")
  const common = {
    id,
    "aria-describedby": describedBy,
    "aria-invalid": error ? true : undefined,
    "aria-required": field.required || undefined,
  }
  // Text the user typed, or nothing (the default applies).
  const text = typeof value === "string" ? value : ""
  const placeholder = fallback ?? undefined

  let input
  switch (control.type) {
    case "text":
      input = (
        <Input
          {...common}
          value={text}
          placeholder={placeholder}
          autoComplete="off"
          spellCheck={false}
          onChange={(e) => {
            onChange(e.target.value)
          }}
        />
      )
      break
    case "multiline":
      input = (
        <Textarea
          {...common}
          value={text}
          placeholder={placeholder}
          spellCheck={false}
          className={cn(control.monospace && "font-mono")}
          onChange={(e) => {
            onChange(e.target.value)
          }}
        />
      )
      break
    case "integer":
      input = (
        <Input
          {...common}
          value={text}
          inputMode="numeric"
          placeholder={placeholder}
          autoComplete="off"
          onChange={(e) => {
            onChange(e.target.value)
          }}
        />
      )
      break
    case "boolean":
      input = (
        <Switch
          {...common}
          checked={typeof value === "boolean" ? value : field.default === true}
          onCheckedChange={(checked) => {
            onChange(checked)
          }}
        />
      )
      break
    case "choice":
      input = (
        <Select
          value={typeof value === "string" && value !== "" ? value : DEFAULT_CHOICE}
          onValueChange={(next) => {
            onChange(next === DEFAULT_CHOICE ? null : next)
          }}
        >
          <SelectTrigger {...common} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={DEFAULT_CHOICE}>
              {fallback ? `Default (${fallback})` : "Not set"}
            </SelectItem>
            {control.options.map((option) => (
              <SelectItem key={option} value={option}>
                {option}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )
      break
    case "unsupported":
      input = (
        <p {...common} className="text-sm text-muted-foreground">
          Settings of kind “{control.kind}” cannot be changed here.
        </p>
      )
  }

  const limits = [
    control.type === "integer" && control.min !== undefined ? `at least ${control.min}` : null,
    control.type === "integer" && control.max !== undefined ? `at most ${control.max}` : null,
    (control.type === "text" || control.type === "multiline") && control.maxLength !== undefined
      ? `up to ${control.maxLength} characters`
      : null,
  ].filter(Boolean)

  return (
    <div className="grid gap-1.5" role="group" aria-labelledby={`${id}-label`}>
      <div className="flex flex-wrap items-center gap-2">
        <label id={`${id}-label`} htmlFor={id} className="text-sm font-medium">
          {field.label}
          {field.required && (
            <span className="text-destructive" aria-hidden>
              {" "}
              *
            </span>
          )}
        </label>
        {field.required && <span className="sr-only">(required)</span>}
        {missing && <Badge variant="destructive">Missing</Badge>}
        {!saved && value === null && fallback !== null && <Badge variant="outline">Default</Badge>}
        {(saved || value !== null) && control.type !== "unsupported" && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="ml-auto h-7"
            aria-label={`Reset ${field.label} to its default`}
            onClick={() => {
              onChange(null)
            }}
          >
            <RotateCcw /> Reset to default
          </Button>
        )}
      </div>
      {input}
      <p id={`${id}-help`} className="text-xs text-muted-foreground">
        {[
          field.description,
          fallback !== null ? `Default: ${fallback}.` : field.required ? "No default." : null,
          limits.length > 0 ? `${limits.join(", ")}.` : null,
        ]
          .filter(Boolean)
          .join(" ")}
      </p>
      {error && (
        <p id={`${id}-error`} className="text-xs text-destructive">
          {error}
        </p>
      )}
    </div>
  )
}

function CredentialRow({
  plugin,
  slot,
  set,
  remove,
  onSlots,
}: {
  plugin: string
  slot: PluginCredentialSlot
  set: (secret: string) => Promise<PluginCredentialSlot[]>
  remove: () => Promise<PluginCredentialSlot[]>
  onSlots: (slots: PluginCredentialSlot[]) => void
}) {
  const id = useId()
  const [mode, setMode] = useState<"idle" | "entering" | "confirmRemove">("idle")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // The secret is only ever in this input's DOM value: never in state.
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (mode === "entering") input.current?.focus()
  }, [mode])

  const submit = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    const field = input.current
    if (!field) return
    const secret = field.value
    // Cleared before anything else happens, whatever the outcome.
    field.value = ""
    const problem = secretProblem(secret)
    if (problem) {
      setError(problem)
      return
    }
    setError(null)
    setBusy(true)
    set(secret)
      .then((slots) => {
        setMode("idle")
        onSlots(slots)
        toast.success(`${slot.label} stored`)
      })
      .catch((e: unknown) => {
        const text = pluginErrorText(e)
        setError(`${text.title}: ${text.message}`)
      })
      .finally(() => {
        setBusy(false)
      })
  }

  const cancel = () => {
    if (input.current) input.current.value = ""
    setError(null)
    setMode("idle")
  }

  const doRemove = () => {
    setBusy(true)
    setError(null)
    remove()
      .then((slots) => {
        setMode("idle")
        onSlots(slots)
        toast.success(`${slot.label} removed`)
      })
      .catch((e: unknown) => {
        const text = pluginErrorText(e)
        setError(`${text.title}: ${text.message}`)
      })
      .finally(() => {
        setBusy(false)
      })
  }

  return (
    <li className="grid gap-2 py-2.5" aria-labelledby={`${id}-label`}>
      <div className="flex flex-wrap items-center gap-2">
        <span id={`${id}-label`} className="text-sm font-medium">
          {slot.label}
        </span>
        <span className="font-mono text-xs text-muted-foreground">{slot.key}</span>
        <Badge variant={slot.set ? "secondary" : "outline"}>{slot.set ? "Set" : "Not set"}</Badge>
        {mode === "idle" && (
          <span className="ml-auto flex gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy}
              aria-label={`${slot.set ? "Replace" : "Set"} ${slot.label}`}
              onClick={() => {
                setError(null)
                setMode("entering")
              }}
            >
              {slot.set ? "Replace…" : "Set…"}
            </Button>
            {slot.set && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={busy}
                aria-label={`Remove ${slot.label}`}
                onClick={() => {
                  setMode("confirmRemove")
                }}
              >
                Remove
              </Button>
            )}
          </span>
        )}
      </div>
      {slot.description && <p className="text-xs text-muted-foreground">{slot.description}</p>}
      <p className="text-xs text-muted-foreground">
        Sent only to{" "}
        {slot.hosts.length > 0 || slot.userOrigin
          ? [
              ...slot.hosts,
              ...(slot.userOrigin
                ? [`the server in the setting “${slot.userOrigin}” once you allow it`]
                : []),
            ].join(", ")
          : "no host"}
        {slot.schemes.length > 0 ? ` (${slot.schemes.join(", ")})` : ""}.
        {slot.set && slot.updatedAt !== undefined ? ` Stored ${formatTime(slot.updatedAt)}.` : ""}
      </p>
      {slot.setupUrl && (
        <p className="text-xs text-muted-foreground">
          Where to get one: <span className="break-all select-text">{slot.setupUrl}</span>
        </p>
      )}
      {!slot.granted && (
        <p className="text-xs text-muted-foreground">
          Its permission is not granted: a stored secret is not used until you grant it.
        </p>
      )}
      {mode === "entering" && (
        <form
          className="flex flex-wrap items-center gap-2"
          aria-label={`Set ${slot.label}`}
          autoComplete="off"
          onSubmit={submit}
        >
          <label htmlFor={`${id}-secret`} className="sr-only">
            {slot.label} for {plugin}
          </label>
          <Input
            ref={input}
            id={`${id}-secret`}
            type="password"
            className="min-w-0 flex-1"
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            data-1p-ignore
            data-lpignore="true"
            defaultValue=""
            disabled={busy}
          />
          <Button type="submit" size="sm" disabled={busy}>
            {busy ? "Storing…" : "Store"}
          </Button>
          <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={cancel}>
            Cancel
          </Button>
        </form>
      )}
      {mode === "confirmRemove" && (
        <div
          className="flex flex-wrap items-center gap-2 text-sm"
          role="group"
          aria-label={`Remove ${slot.label}?`}
        >
          <span>Remove the stored {slot.label}? The plugin cannot use it afterwards.</span>
          <Button type="button" variant="destructive" size="sm" disabled={busy} onClick={doRemove}>
            Remove
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => {
              setMode("idle")
            }}
          >
            Keep it
          </Button>
        </div>
      )}
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </li>
  )
}
