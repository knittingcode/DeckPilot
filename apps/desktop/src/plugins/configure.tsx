// Configure: a plugin's settings and credentials, against the agent's
// `getPluginSettings` / `setPluginSettings` and `listPluginCredentials` /
// `setPluginCredential` / `deletePluginCredential`.
//
// Credentials: the agent never returns a secret, only whether one is
// stored. The secret the user types lives in an uncontrolled password
// input (the DOM only, never React state, props or context); on submit it
// is read once, the input is cleared at once, and it goes to the agent.
// Nothing here logs it, puts it into browser storage, or shows it again.

import { RotateCcw, Settings2 } from "lucide-react"
import { useEffect, useId, useRef, useState, type SyntheticEvent } from "react"
import { toast } from "sonner"

import { FieldError, ModalShell } from "@/components/dialogs"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
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
import { i18n, useTranslation } from "@/i18n"
import { cn } from "@/lib/utils"

import { pluginErrorText, type PluginClient, type PluginErrorText } from "./api"
import { Notice } from "./components"
import { errorSentence } from "./dialog-error"
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
  const { t } = useTranslation()
  return (
    <ModalShell
      presentation="dialog"
      size="medium"
      icon={Settings2}
      title={t("plugins:configure.title", { name: item.summary.name })}
      description={t("plugins:configure.description", { version: item.summary.version })}
      onClose={onClose}
    >
      {/* The settings form has its own actions: it is also shown outside a dialog. */}
      <PluginConfiguration
        id={item.summary.id}
        name={item.summary.name}
        client={client}
        onClose={onClose}
        onChanged={onChanged}
      />
    </ModalShell>
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
  const { t } = useTranslation()
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
            {t("plugins:configure.settings")}
          </h4>
          {settings.status === "loading" ? (
            <p className="text-sm text-muted-foreground">
              {t("plugins:configure.loadingSettings")}
            </p>
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
            {t("plugins:configure.credentials")}
          </h4>
          {slots.status === "loading" ? (
            <p className="text-sm text-muted-foreground">
              {t("plugins:configure.loadingCredentials")}
            </p>
          ) : slots.status === "error" ? (
            <ErrorText error={slots.error} />
          ) : (
            <>
              <p className="text-xs text-muted-foreground">
                {t("plugins:configure.credentialsHint")}
              </p>
              <ul
                className="grid divide-y border-y"
                aria-label={t("plugins:configure.credentialsOf", { name })}
              >
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
        <p className="text-sm text-muted-foreground">{t("plugins:configure.nothing")}</p>
      )}
    </>
  )
}

function ErrorText({ error }: { error: PluginErrorText }) {
  const id = useId()
  return <FieldError id={id} message={errorSentence(error)} />
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
  const { t } = useTranslation()
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
        toast.success(i18n.t("plugins:configure.saved"))
      })
      .catch((error: unknown) => {
        const text = pluginErrorText(error)
        if (text.reason === "invalidData") {
          const problems = settingsProblems(text.message, settings.fields)
          setErrors(problems.fields)
          setFormErrors(problems.form)
        } else {
          setFormErrors([errorSentence(text)])
        }
      })
      .finally(() => {
        setSaving(false)
      })
  }

  return (
    <form
      className="grid gap-4"
      aria-label={t("plugins:configure.settings")}
      noValidate
      onSubmit={submit}
    >
      {missing.length > 0 && (
        <Notice title={t("plugins:settings.missing.title")}>
          <p>
            {t("plugins:settings.missing.text", {
              count: missing.length,
              settings: missing.join(", "),
            })}
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
      <p className="text-xs text-muted-foreground">{t("plugins:settings.notSecret")}</p>
      {/* Plain layout, not a dialog part: the form is also shown outside a dialog. */}
      <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
        <Button type="button" variant="outline" onClick={onCancel}>
          {t("common:actions.cancel")}
        </Button>
        <Button type="submit" disabled={saving || !dirty}>
          {saving ? t("plugins:settings.saving") : t("plugins:settings.save")}
        </Button>
      </div>
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
  const { t } = useTranslation()
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
              {fallback
                ? t("plugins:settings.defaultChoice", { value: fallback })
                : t("plugins:settings.notSet")}
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
          {t("plugins:settings.unsupported", { kind: control.kind })}
        </p>
      )
  }

  const limits = [
    control.type === "integer" && control.min !== undefined
      ? t("plugins:settings.limit.min", { min: control.min })
      : null,
    control.type === "integer" && control.max !== undefined
      ? t("plugins:settings.limit.max", { max: control.max })
      : null,
    (control.type === "text" || control.type === "multiline") && control.maxLength !== undefined
      ? t("plugins:settings.limit.length", { count: control.maxLength })
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
        {field.required && <span className="sr-only">{t("plugins:dialog.required")}</span>}
        {missing && <Badge variant="destructive">{t("plugins:settings.missingBadge")}</Badge>}
        {!saved && value === null && fallback !== null && (
          <Badge variant="outline">{t("plugins:settings.defaultBadge")}</Badge>
        )}
        {(saved || value !== null) && control.type !== "unsupported" && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="ml-auto h-7"
            aria-label={t("plugins:settings.resetNamed", { label: field.label })}
            onClick={() => {
              onChange(null)
            }}
          >
            <RotateCcw /> {t("plugins:settings.reset")}
          </Button>
        )}
      </div>
      {input}
      <p id={`${id}-help`} className="text-xs text-muted-foreground">
        {[
          field.description,
          fallback !== null
            ? t("plugins:settings.defaultIs", { value: fallback })
            : field.required
              ? t("plugins:settings.noDefault")
              : null,
          limits.length > 0 ? t("plugins:settings.limits", { limits: limits.join(", ") }) : null,
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
  const { t } = useTranslation()
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
        toast.success(i18n.t("plugins:credentials.stored", { label: slot.label }))
      })
      .catch((e: unknown) => {
        setError(errorSentence(pluginErrorText(e), false))
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
        toast.success(i18n.t("plugins:credentials.removed", { label: slot.label }))
      })
      .catch((e: unknown) => {
        setError(errorSentence(pluginErrorText(e), false))
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
        <Badge variant={slot.set ? "secondary" : "outline"}>
          {slot.set ? t("plugins:credentials.set") : t("plugins:settings.notSet")}
        </Badge>
        {mode === "idle" && (
          <span className="ml-auto flex gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy}
              aria-label={
                slot.set
                  ? t("plugins:credentials.replaceNamed", { label: slot.label })
                  : t("plugins:credentials.setNamed", { label: slot.label })
              }
              onClick={() => {
                setError(null)
                setMode("entering")
              }}
            >
              {slot.set ? t("plugins:credentials.replace") : t("plugins:credentials.setButton")}
            </Button>
            {slot.set && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={busy}
                aria-label={t("plugins:credentials.removeNamed", { label: slot.label })}
                onClick={() => {
                  setMode("confirmRemove")
                }}
              >
                {t("plugins:credentials.remove")}
              </Button>
            )}
          </span>
        )}
      </div>
      {slot.description && <p className="text-xs text-muted-foreground">{slot.description}</p>}
      <p className="text-xs text-muted-foreground">{sentTo(slot)}</p>
      {slot.setupUrl && (
        <p className="text-xs text-muted-foreground">
          {t("plugins:credentials.setupUrl")}{" "}
          <span className="break-all select-text">{slot.setupUrl}</span>
        </p>
      )}
      {!slot.granted && (
        <p className="text-xs text-muted-foreground">{t("plugins:credentials.notGranted")}</p>
      )}
      {mode === "entering" && (
        <form
          className="flex flex-wrap items-center gap-2"
          aria-label={t("plugins:credentials.setNamed", { label: slot.label })}
          autoComplete="off"
          onSubmit={submit}
        >
          <label htmlFor={`${id}-secret`} className="sr-only">
            {t("plugins:credentials.inputLabel", { label: slot.label, plugin })}
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
            {busy ? t("plugins:credentials.storing") : t("plugins:credentials.store")}
          </Button>
          <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={cancel}>
            {t("common:actions.cancel")}
          </Button>
        </form>
      )}
      {mode === "confirmRemove" && (
        <div
          className="flex flex-wrap items-center gap-2 text-sm"
          role="group"
          aria-label={t("plugins:credentials.removeAsk", { label: slot.label })}
        >
          <span>{t("plugins:credentials.removeConfirm", { label: slot.label })}</span>
          <Button type="button" variant="destructive" size="sm" disabled={busy} onClick={doRemove}>
            {t("plugins:credentials.remove")}
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
            {t("plugins:credentials.keep")}
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

/** Where a credential slot's secret goes, and when it was stored. */
function sentTo(slot: PluginCredentialSlot): string {
  const targets = [
    ...slot.hosts,
    ...(slot.userOrigin
      ? [i18n.t("plugins:credentials.userOrigin", { setting: slot.userOrigin })]
      : []),
  ]
  const where = targets.length > 0 ? targets.join(", ") : i18n.t("plugins:credentials.noHost")
  const sent =
    slot.schemes.length > 0
      ? i18n.t("plugins:credentials.sentToSchemes", {
          targets: where,
          schemes: slot.schemes.join(", "),
        })
      : i18n.t("plugins:credentials.sentTo", { targets: where })
  return slot.set && slot.updatedAt !== undefined
    ? `${sent} ${i18n.t("plugins:credentials.storedAt", { time: formatTime(slot.updatedAt) })}`
    : sent
}
