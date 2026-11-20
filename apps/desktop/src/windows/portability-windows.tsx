import { emitTo } from "@tauri-apps/api/event"
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog"
import { Download, FileCheck2, TriangleAlert, Upload } from "lucide-react"
import { useEffect, useState } from "react"

import { SensitiveOperations } from "@/components/authentication"
import { ModalShell } from "@/components/modal-shell"
import { NewPasswordFields, PasswordField } from "@/components/password-field"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { NAVIGATE_EVENT, api } from "@/lib/api"
import { failureMessage } from "@/lib/failure"
import { RESOURCE_KIND_LABELS } from "@/lib/format"
import { newPasswordReady } from "@/lib/passwords"
import { isCancelled, useSensitive } from "@/lib/sensitive"
import type {
  ConflictResolution,
  ExportResult,
  ImportMode,
  ImportPreview,
  ImportSummary,
  ProfilesView,
} from "@/lib/types"
import { type TranslationKey, i18n, useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"

import { useWindow } from "./window-context"

/**
 * Export and import of the portable configuration, each in its own native
 * window. The export password is independent of how the database is
 * protected. Imports are previewed first and applied in one transaction;
 * the main window refreshes when the backend reports the change.
 */

/** The file type of the native file dialogs, named in the window's language. */
const filters = () => [{ name: i18n.t("settings:portability.fileType"), extensions: ["deckpilot"] }]

/** Loads what a window needs before it can show its form. */
function useLoaded<T>(load: () => Promise<T>) {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    load().then(setData, (e: unknown) => {
      setError(
        failureMessage(
          i18n.t("settings:portability.profilesUnreadable"),
          e,
          i18n.t("settings:portability.closeAndRetry"),
        ),
      )
    })
    // Loaded once per window.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  return { data, error }
}

export function ExportWindow() {
  const { t } = useTranslation()
  const loaded = useLoaded(() => Promise.all([api.listProfiles(), api.securityStatus()]))
  const { close } = useWindow()
  if (!loaded.data) {
    return (
      <ModalShell
        size="medium"
        icon={Download}
        title={t("settings:portability.export.title")}
        loading={!loaded.error}
        error={loaded.error}
        onClose={close}
      />
    )
  }
  const [profiles, security] = loaded.data
  return (
    <SensitiveOperations verification={security.verification}>
      <ExportForm profiles={profiles} />
    </SensitiveOperations>
  )
}

function ExportForm({ profiles }: { profiles: ProfilesView }) {
  const { t } = useTranslation()
  const { close } = useWindow()
  const sensitive = useSensitive()
  const [selected, setSelected] = useState(profiles.profiles.map((p) => p.id))
  const [includeModules, setIncludeModules] = useState(true)
  const [includeSettings, setIncludeSettings] = useState(true)
  const [password, setPassword] = useState("")
  const [confirmation, setConfirmation] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<ExportResult | null>(null)
  const ready = selected.length > 0 && newPasswordReady(password, confirmation)

  const submit = async () => {
    if (!ready) return
    setError(null)
    const path = await saveDialog({
      defaultPath: "deckpilot-configuration.deckpilot",
      filters: filters(),
    })
    if (!path) return
    setBusy(true)
    try {
      setResult(
        await sensitive(
          () =>
            api.exportConfiguration({
              path,
              password,
              confirmation,
              profiles: selected.length === profiles.profiles.length ? null : selected,
              includeModuleSettings: includeModules,
              includeAppSettings: includeSettings,
            }),
          t("settings:portability.export.purpose"),
        ),
      )
      setPassword("")
      setConfirmation("")
    } catch (e) {
      if (!isCancelled(e)) {
        setError(failureMessage(t("settings:portability.export.failed"), e))
      }
    } finally {
      setBusy(false)
    }
  }

  if (result) {
    return (
      <ModalShell
        size="medium"
        icon={FileCheck2}
        title={t("settings:portability.export.doneTitle")}
        description={t("settings:portability.export.doneDescription", {
          profiles: t("settings:portability.count.profiles", { count: result.profiles }),
          modules: t("settings:portability.count.moduleSettings", { count: result.modules }),
        })}
        onClose={close}
        primary={{ label: t("settings:shared.done"), onClick: close }}
      >
        <p className="font-mono text-xs break-all select-text">{result.path}</p>
        <p className="text-xs text-muted-foreground">{t("settings:portability.export.keepFile")}</p>
      </ModalShell>
    )
  }

  return (
    <ModalShell
      size="medium"
      icon={Download}
      title={t("settings:portability.export.title")}
      description={t("settings:portability.export.description")}
      error={error}
      onClose={close}
      onSubmit={() => void submit()}
      secondary={{ label: t("common:actions.cancel"), onClick: close }}
      primary={{
        label: t("settings:portability.export.action"),
        loadingLabel: t("settings:portability.export.encrypting"),
        loading: busy,
        disabled: !ready,
      }}
    >
      <fieldset className="grid gap-2">
        <legend className="mb-1 text-sm font-medium">{t("settings:pages.profiles.title")}</legend>
        {profiles.profiles.length === 0 && (
          <p className="text-sm text-muted-foreground">
            {t("settings:portability.export.noProfiles")}
          </p>
        )}
        {profiles.profiles.map((profile) => (
          <label key={profile.id} className="flex items-center gap-2 text-sm">
            <Checkbox
              checked={selected.includes(profile.id)}
              onCheckedChange={(checked) => {
                setSelected(
                  checked === true
                    ? [...selected, profile.id]
                    : selected.filter((id) => id !== profile.id),
                )
              }}
            />
            {profile.name}
          </label>
        ))}
      </fieldset>
      <div className="grid gap-2">
        <label className="flex items-center gap-2 text-sm">
          <Checkbox
            checked={includeModules}
            onCheckedChange={(checked) => {
              setIncludeModules(checked === true)
            }}
          />
          {t("settings:portability.export.includeModules")}
        </label>
        <label className="flex items-center gap-2 text-sm">
          <Checkbox
            checked={includeSettings}
            onCheckedChange={(checked) => {
              setIncludeSettings(checked === true)
            }}
          />
          {t("settings:portability.export.includeSettings")}
        </label>
      </div>
      <NewPasswordFields
        password={password}
        confirmation={confirmation}
        onPassword={setPassword}
        onConfirmation={setConfirmation}
        idPrefix="export-password"
      />
      <p className="text-xs text-muted-foreground">{t("settings:portability.export.excluded")}</p>
    </ModalShell>
  )
}

type ImportStep =
  | { step: "file" }
  | { step: "password"; path: string }
  | { step: "preview"; token: number; preview: ImportPreview }
  | { step: "done"; summary: ImportSummary; preview: ImportPreview }

export function ImportWindow() {
  const { close } = useWindow()
  const [state, setState] = useState<ImportStep>({ step: "file" })
  const cancel = () => {
    if (state.step === "preview") void api.cancelImport()
    close()
  }
  switch (state.step) {
    case "file":
      return (
        <ChooseFile
          onClose={cancel}
          onChosen={(path) => {
            setState({ step: "password", path })
          }}
        />
      )
    case "password":
      return (
        <ImportPassword
          path={state.path}
          onClose={cancel}
          onBack={() => {
            setState({ step: "file" })
          }}
          onPreview={(token, preview) => {
            setState({ step: "preview", token, preview })
          }}
        />
      )
    case "preview":
      return (
        <ImportChoices
          token={state.token}
          preview={state.preview}
          onClose={cancel}
          onDone={(summary) => {
            setState({ step: "done", summary, preview: state.preview })
          }}
        />
      )
    case "done":
      return <ImportDone summary={state.summary} preview={state.preview} onClose={close} />
  }
}

function ChooseFile({
  onChosen,
  onClose,
}: {
  onChosen: (path: string) => void
  onClose: () => void
}) {
  const { t } = useTranslation()
  const [error, setError] = useState<string | null>(null)
  return (
    <ModalShell
      size="medium"
      icon={Upload}
      title={t("settings:portability.import.title")}
      description={t("settings:portability.import.chooseDescription")}
      error={error}
      onClose={onClose}
      onSubmit={() => {
        openDialog({ multiple: false, directory: false, filters: filters() }).then(
          (path) => {
            if (typeof path === "string") onChosen(path)
          },
          (e: unknown) => {
            setError(failureMessage(t("settings:portability.import.chooseFailed"), e))
          },
        )
      }}
      secondary={{ label: t("common:actions.cancel"), onClick: onClose }}
      primary={{ label: t("settings:portability.import.chooseFile") }}
    />
  )
}

function ImportPassword({
  path,
  onPreview,
  onBack,
  onClose,
}: {
  path: string
  onPreview: (token: number, preview: ImportPreview) => void
  onBack: () => void
  onClose: () => void
}) {
  const { t } = useTranslation()
  const [password, setPassword] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  return (
    <ModalShell
      size="medium"
      icon={Upload}
      title={t("settings:portability.import.title")}
      description={<span className="font-mono text-xs break-all">{path}</span>}
      error={error}
      onClose={onClose}
      onSubmit={() => {
        if (!password) return
        setBusy(true)
        setError(null)
        api
          .previewImport(path, password)
          .then((result) => {
            onPreview(result.token, result.preview)
          })
          .catch((e: unknown) => {
            setError(
              failureMessage(
                t("settings:portability.import.openFailed"),
                e,
                t("settings:portability.import.openFailedNext"),
              ),
            )
            setPassword("")
          })
          .finally(() => {
            setBusy(false)
          })
      }}
      secondary={{ label: t("common:actions.back"), onClick: onBack }}
      primary={{
        label: t("settings:portability.import.showContents"),
        loadingLabel: t("settings:portability.import.decrypting"),
        loading: busy,
        disabled: !password,
      }}
    >
      <PasswordField
        id="import-password"
        label={t("settings:portability.import.password")}
        value={password}
        onChange={setPassword}
        autoComplete="current-password"
        autoFocus
        invalid={Boolean(error)}
      />
      <p className="text-xs text-muted-foreground">{t("settings:portability.import.nothingYet")}</p>
    </ModalShell>
  )
}

const RESOLUTIONS: { value: ConflictResolution; label: TranslationKey }[] = [
  { value: "keepBoth", label: "settings:portability.import.resolution.keepBoth" },
  { value: "replace", label: "settings:portability.import.resolution.replace" },
  { value: "skip", label: "settings:portability.import.resolution.skip" },
]

function ImportChoices({
  token,
  preview,
  onDone,
  onClose,
}: {
  token: number
  preview: ImportPreview
  onDone: (summary: ImportSummary) => void
  onClose: () => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const [mode, setMode] = useState<ImportMode>("merge")
  const [selected, setSelected] = useState(preview.profiles.map((p) => p.id))
  const [conflicts, setConflicts] = useState<Record<string, ConflictResolution>>({})
  const ready = preview.modules.some((m) => m.status === "ready")
  const [moduleSettings, setModuleSettings] = useState(ready)
  const variables = preview.variables
  // Global variable values travel with the application settings.
  const hasSettings = Boolean(preview.settings.updates) || variables.globalValues > 0
  const [appSettings, setAppSettings] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const nothing = selected.length === 0 && !moduleSettings && !appSettings

  return (
    <ModalShell
      size="medium"
      icon={Upload}
      title={t("settings:portability.import.reviewTitle")}
      description={t("settings:portability.import.reviewDescription", {
        version: preview.appVersion,
        date: format.dateTime(preview.exportedAt * 1000),
      })}
      error={error}
      onClose={onClose}
      onSubmit={() => {
        setBusy(true)
        setError(null)
        api
          .applyImport(token, { mode, profiles: selected, conflicts, moduleSettings, appSettings })
          .then(onDone)
          .catch((e: unknown) => {
            setError(failureMessage(t("settings:portability.import.failed"), e))
          })
          .finally(() => {
            setBusy(false)
          })
      }}
      secondary={{ label: t("common:actions.cancel"), onClick: onClose }}
      primary={{
        label: t("settings:backup.import"),
        loadingLabel: t("settings:portability.import.importing"),
        loading: busy,
        disabled: nothing,
        variant: mode === "replace" ? "destructive" : "default",
      }}
    >
      <div className="grid gap-2">
        <Label htmlFor="import-mode">{t("settings:portability.import.mode")}</Label>
        <Select
          value={mode}
          onValueChange={(value) => {
            setMode(value as ImportMode)
          }}
        >
          <SelectTrigger id="import-mode" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="merge">{t("settings:portability.import.merge")}</SelectItem>
            <SelectItem value="replace">{t("settings:portability.import.replace")}</SelectItem>
          </SelectContent>
        </Select>
        {mode === "replace" && preview.localProfiles.length > 0 && (
          <Alert variant="destructive">
            <TriangleAlert />
            <AlertTitle>{t("settings:portability.import.removedTitle")}</AlertTitle>
            <AlertDescription>
              {t("settings:portability.import.removedDescription", {
                profiles: format.list(
                  preview.localProfiles.map(([, name]) => name),
                  "unit",
                ),
              })}
            </AlertDescription>
          </Alert>
        )}
      </div>

      <fieldset className="grid gap-2">
        <legend className="mb-1 text-sm font-medium">{t("settings:pages.profiles.title")}</legend>
        {preview.profiles.map((profile) => (
          <div key={profile.id} className="grid gap-2 rounded-md border p-3">
            <label className="flex items-center gap-2 text-sm">
              <Checkbox
                checked={selected.includes(profile.id)}
                onCheckedChange={(checked) => {
                  setSelected(
                    checked === true
                      ? [...selected, profile.id]
                      : selected.filter((id) => id !== profile.id),
                  )
                }}
              />
              <span className="font-medium">{profile.name}</span>
              <span className="text-xs text-muted-foreground">
                {[
                  t("settings:portability.count.actions", { count: profile.actions }),
                  t("settings:portability.count.workflows", { count: profile.workflows }),
                  t("settings:portability.count.projects", { count: profile.projects }),
                  t("settings:portability.count.layouts", { count: profile.layouts }),
                ].join(" · ")}
              </span>
            </label>
            {profile.conflict?.type === "sameId" && mode === "merge" && (
              <div className="flex flex-wrap items-center gap-2 pl-6 text-sm">
                <Badge variant="outline">
                  {t("settings:portability.import.alreadyHere", { name: profile.conflict.name })}
                </Badge>
                <Select
                  value={conflicts[profile.id] ?? "keepBoth"}
                  onValueChange={(value) => {
                    setConflicts({ ...conflicts, [profile.id]: value as ConflictResolution })
                  }}
                >
                  <SelectTrigger
                    className="h-8 w-56"
                    aria-label={t("settings:portability.import.resolutionLabel", {
                      profile: profile.name,
                    })}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {RESOLUTIONS.map((r) => (
                      <SelectItem key={r.value} value={r.value}>
                        {t(r.label)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}
            {profile.conflict?.type === "sameName" && mode === "merge" && (
              <p className="pl-6 text-xs text-muted-foreground">
                {t("settings:portability.import.sameName")}
              </p>
            )}
          </div>
        ))}
      </fieldset>

      {preview.modules.length > 0 && (
        <div className="grid gap-2">
          <label className="flex items-center gap-2 text-sm font-medium">
            <Checkbox
              checked={moduleSettings}
              disabled={!ready}
              onCheckedChange={(checked) => {
                setModuleSettings(checked === true)
              }}
            />
            {t("settings:portability.import.applyModules")}
          </label>
          <ul className="grid gap-1 pl-6 text-sm">
            {preview.modules.map((module) => (
              <li key={module.id} className="flex flex-wrap items-center gap-2">
                <span>{module.name ?? module.id}</span>
                <Badge variant={module.status === "ready" ? "secondary" : "outline"}>
                  {module.status === "ready"
                    ? module.enabled
                      ? t("settings:modules.state.enabled")
                      : t("settings:modules.state.disabled")
                    : module.status === "missing"
                      ? t("settings:modules.state.notInstalled")
                      : t("settings:portability.import.incompatible")}
                </Badge>
                {module.problem && (
                  <span className="text-xs text-muted-foreground">{module.problem}</span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      {hasSettings && (
        <label className="flex items-center gap-2 text-sm font-medium">
          <Checkbox
            checked={appSettings}
            onCheckedChange={(checked) => {
              setAppSettings(checked === true)
            }}
          />
          {t("settings:portability.import.applySettings", {
            content: format.list(
              settingsContent(preview).map((key) => t(key)),
              "unit",
            ),
          })}
        </label>
      )}

      {(variables.new.length > 0 || variables.existing.length > 0 || variables.values > 0) && (
        <div className="grid gap-1 text-sm" aria-label={t("settings:portability.variables.title")}>
          <h2 className="font-medium">{t("settings:portability.variables.title")}</h2>
          {variables.new.length > 0 && (
            <p>
              {t("settings:portability.variables.new", {
                names: format.list(variables.new, "unit"),
              })}
            </p>
          )}
          {variables.existing.length > 0 && (
            <p>
              {t("settings:portability.variables.existing", {
                names: format.list(variables.existing, "unit"),
              })}
            </p>
          )}
          <p className="text-xs text-muted-foreground">
            {variables.globalValues > 0
              ? t("settings:portability.variables.valuesWithGlobal", {
                  values: t("settings:portability.count.values", { count: variables.values }),
                  global: t("settings:portability.count.globalValues", {
                    count: variables.globalValues,
                  }),
                })
              : t("settings:portability.variables.values", {
                  values: t("settings:portability.count.values", { count: variables.values }),
                })}
          </p>
        </div>
      )}
      {variables.incompatible.length > 0 && (
        <Alert>
          <TriangleAlert />
          <AlertTitle>{t("settings:portability.variables.incompatibleTitle")}</AlertTitle>
          <AlertDescription>
            {t("settings:portability.variables.incompatibleDescription", {
              names: format.list(variables.incompatible, "unit"),
            })}
          </AlertDescription>
        </Alert>
      )}

      {preview.unknownActionTypes.length > 0 && (
        <Alert>
          <TriangleAlert />
          <AlertTitle>{t("settings:portability.import.unknownTypesTitle")}</AlertTitle>
          <AlertDescription>
            {t("settings:portability.import.unknownTypesDescription", {
              types: format.list(preview.unknownActionTypes, "unit"),
            })}
          </AlertDescription>
        </Alert>
      )}
      {preview.missingResources.length > 0 && (
        <Alert>
          <TriangleAlert />
          <AlertTitle>{t("settings:portability.import.missingTitle")}</AlertTitle>
          <AlertDescription>
            {t("settings:portability.import.missingDescription", {
              resources: format.list(
                preview.missingResources.map((r) => r.key),
                "unit",
              ),
            })}
          </AlertDescription>
        </Alert>
      )}
      {preview.warnings.length > 0 && (
        <ul className="list-disc pl-5 text-xs text-muted-foreground">
          {preview.warnings.map((warning) => (
            <li key={warning}>{warning}</li>
          ))}
        </ul>
      )}
    </ModalShell>
  )
}

/** What "application settings" means for this export. */
function settingsContent(preview: ImportPreview): TranslationKey[] {
  const parts: TranslationKey[] = []
  if (preview.settings.updates) parts.push("settings:portability.import.settingsUpdates")
  if (preview.variables.globalValues > 0) {
    parts.push("settings:portability.import.settingsGlobalValues")
  }
  return parts
}

function ImportDone({
  summary,
  preview,
  onClose,
}: {
  summary: ImportSummary
  preview: ImportPreview
  onClose: () => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const missing = preview.missingResources
  const outcome = [
    t("settings:portability.done.imported", { count: summary.imported.length }),
    summary.removed.length > 0
      ? t("settings:portability.done.replaced", { count: summary.removed.length })
      : "",
    summary.skipped.length > 0
      ? t("settings:portability.done.skipped", { count: summary.skipped.length })
      : "",
  ].filter(Boolean)
  const values = t("settings:portability.count.values", { count: summary.variableValues })
  return (
    <ModalShell
      size="medium"
      icon={FileCheck2}
      title={t("settings:portability.done.title")}
      description={t("settings:portability.done.description", {
        outcome: format.list(outcome, "unit"),
      })}
      onClose={onClose}
      secondary={
        missing.length > 0
          ? {
              label: t("settings:portability.done.mapResources"),
              onClick: () => {
                void emitTo("main", NAVIGATE_EVENT, "projects/projects").then(() =>
                  api.showMainWindow(),
                )
                onClose()
              },
            }
          : undefined
      }
      primary={{ label: t("settings:shared.done"), onClick: onClose }}
    >
      {summary.renamed.length > 0 && (
        <p className="text-sm">
          {t("settings:portability.done.renamed", {
            profiles: format.list(summary.renamed, "unit"),
          })}
        </p>
      )}
      {summary.settings.length > 0 && (
        <p className="text-sm">
          {t("settings:portability.done.settings", {
            settings: format.list(summary.settings, "unit"),
          })}
        </p>
      )}
      {(summary.variables.length > 0 || summary.variableValues > 0) && (
        <p className="text-sm">
          {summary.variables.length > 0
            ? t("settings:portability.done.variablesCreated", {
                names: format.list(summary.variables, "unit"),
                values,
              })
            : t("settings:portability.done.variables", { values })}
        </p>
      )}
      {summary.skippedValues.length > 0 && (
        <div className="grid gap-1">
          <h2 className="text-sm font-medium">{t("settings:portability.done.skippedValues")}</h2>
          <ul
            className="list-disc pl-5 text-sm"
            aria-label={t("settings:portability.done.skippedValuesList")}
          >
            {summary.skippedValues.map((reason) => (
              <li key={reason}>{reason}</li>
            ))}
          </ul>
        </div>
      )}
      {missing.length > 0 && (
        <div className="grid gap-2">
          <h2 className="text-sm font-medium">{t("settings:portability.done.setUp")}</h2>
          <ul className="grid gap-1 text-sm">
            {missing.map((resource) => (
              <li key={resource.key}>
                <span className="font-mono">{resource.key}</span>
                {resource.kind && (
                  <span className="text-muted-foreground">
                    {" "}
                    · {t(RESOURCE_KIND_LABELS[resource.kind])}
                  </span>
                )}
                <span className="text-xs text-muted-foreground">
                  {" "}
                  {t("settings:portability.done.usedBy", {
                    profiles: format.list(resource.profiles, "unit"),
                  })}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </ModalShell>
  )
}
