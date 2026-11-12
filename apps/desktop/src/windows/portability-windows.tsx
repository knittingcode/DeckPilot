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

import { useWindow } from "./window-context"

/**
 * Export and import of the portable configuration, each in its own native
 * window. The export password is independent of how the database is
 * protected. Imports are previewed first and applied in one transaction;
 * the main window refreshes when the backend reports the change.
 */

const FILTERS = [{ name: "DeckPilot configuration", extensions: ["deckpilot"] }]

/** Loads what a window needs before it can show its form. */
function useLoaded<T>(load: () => Promise<T>) {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    load().then(setData, (e: unknown) => {
      setError(
        failureMessage("The profiles could not be loaded", e, "Close the window and try again"),
      )
    })
    // Loaded once per window.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  return { data, error }
}

export function ExportWindow() {
  const loaded = useLoaded(() => Promise.all([api.listProfiles(), api.securityStatus()]))
  const { close } = useWindow()
  if (!loaded.data) {
    return (
      <ModalShell
        size="medium"
        icon={Download}
        title="Export configuration"
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
      filters: FILTERS,
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
          "export your configuration",
        ),
      )
      setPassword("")
      setConfirmation("")
    } catch (e) {
      if (!isCancelled(e)) setError(failureMessage("The configuration was not exported", e))
    } finally {
      setBusy(false)
    }
  }

  if (result) {
    return (
      <ModalShell
        size="medium"
        icon={FileCheck2}
        title="Configuration exported"
        description={`${result.profiles} profile(s) and ${result.modules} module setting(s), encrypted with your export password.`}
        onClose={close}
        primary={{ label: "Done", onClick: close }}
      >
        <p className="font-mono text-xs break-all select-text">{result.path}</p>
        <p className="text-xs text-muted-foreground">
          Keep the file and its password; you need both to import it. This file is your backup: the
          key of this computer is not.
        </p>
      </ModalShell>
    )
  }

  return (
    <ModalShell
      size="medium"
      icon={Download}
      title="Export configuration"
      description="The file is encrypted with a password of its own, independent of how DeckPilot is protected on this computer."
      error={error}
      onClose={close}
      onSubmit={() => void submit()}
      secondary={{ label: "Cancel", onClick: close }}
      primary={{
        label: "Choose file and export",
        loadingLabel: "Encrypting…",
        loading: busy,
        disabled: !ready,
      }}
    >
      <fieldset className="grid gap-2">
        <legend className="mb-1 text-sm font-medium">Profiles</legend>
        {profiles.profiles.length === 0 && (
          <p className="text-sm text-muted-foreground">There are no profiles yet.</p>
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
          Include module settings
        </label>
        <label className="flex items-center gap-2 text-sm">
          <Checkbox
            checked={includeSettings}
            onCheckedChange={(checked) => {
              setIncludeSettings(checked === true)
            }}
          />
          Include application settings (updates)
        </label>
      </div>
      <NewPasswordFields
        password={password}
        confirmation={confirmation}
        onPassword={setPassword}
        onConfirmation={setConfirmation}
        idPrefix="export-password"
      />
      <p className="text-xs text-muted-foreground">
        Paths and other settings of this computer, keys, passwords and the security settings are
        never included.
      </p>
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
  const [error, setError] = useState<string | null>(null)
  return (
    <ModalShell
      size="medium"
      icon={Upload}
      title="Import configuration"
      description="Choose a DeckPilot export (.deckpilot). Nothing is changed until you have reviewed its contents."
      error={error}
      onClose={onClose}
      onSubmit={() => {
        openDialog({ multiple: false, directory: false, filters: FILTERS }).then(
          (path) => {
            if (typeof path === "string") onChosen(path)
          },
          (e: unknown) => {
            setError(failureMessage("The file could not be chosen", e))
          },
        )
      }}
      secondary={{ label: "Cancel", onClick: onClose }}
      primary={{ label: "Choose file…" }}
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
  const [password, setPassword] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  return (
    <ModalShell
      size="medium"
      icon={Upload}
      title="Import configuration"
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
                "The file could not be opened",
                e,
                "Check the export password and that the file is a DeckPilot export",
              ),
            )
            setPassword("")
          })
          .finally(() => {
            setBusy(false)
          })
      }}
      secondary={{ label: "Back", onClick: onBack }}
      primary={{
        label: "Show contents",
        loadingLabel: "Decrypting…",
        loading: busy,
        disabled: !password,
      }}
    >
      <PasswordField
        id="import-password"
        label="Export password"
        value={password}
        onChange={setPassword}
        autoComplete="current-password"
        autoFocus
        invalid={Boolean(error)}
      />
      <p className="text-xs text-muted-foreground">
        Nothing is changed yet: you see what the file contains first.
      </p>
    </ModalShell>
  )
}

const RESOLUTIONS: { value: ConflictResolution; label: string }[] = [
  { value: "keepBoth", label: "Import as a copy" },
  { value: "replace", label: "Replace the local profile" },
  { value: "skip", label: "Skip" },
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
  const [mode, setMode] = useState<ImportMode>("merge")
  const [selected, setSelected] = useState(preview.profiles.map((p) => p.id))
  const [conflicts, setConflicts] = useState<Record<string, ConflictResolution>>({})
  const ready = preview.modules.some((m) => m.status === "ready")
  const [moduleSettings, setModuleSettings] = useState(ready)
  const hasSettings = Boolean(preview.settings.updates)
  const [appSettings, setAppSettings] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const nothing = selected.length === 0 && !moduleSettings && !appSettings

  return (
    <ModalShell
      size="medium"
      icon={Upload}
      title="Review the import"
      description={`Exported by DeckPilot ${preview.appVersion} on ${new Date(preview.exportedAt * 1000).toLocaleString()}. Imported workflows do not run and imported profiles are not activated.`}
      error={error}
      onClose={onClose}
      onSubmit={() => {
        setBusy(true)
        setError(null)
        api
          .applyImport(token, { mode, profiles: selected, conflicts, moduleSettings, appSettings })
          .then(onDone)
          .catch((e: unknown) => {
            setError(failureMessage("Nothing was imported", e))
          })
          .finally(() => {
            setBusy(false)
          })
      }}
      secondary={{ label: "Cancel", onClick: onClose }}
      primary={{
        label: "Import",
        loadingLabel: "Importing…",
        loading: busy,
        disabled: nothing,
        variant: mode === "replace" ? "destructive" : "default",
      }}
    >
      <div className="grid gap-2">
        <Label htmlFor="import-mode">How to import</Label>
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
            <SelectItem value="merge">Add to my configuration</SelectItem>
            <SelectItem value="replace">Replace my profiles</SelectItem>
          </SelectContent>
        </Select>
        {mode === "replace" && preview.localProfiles.length > 0 && (
          <Alert variant="destructive">
            <TriangleAlert />
            <AlertTitle>These local profiles will be removed</AlertTitle>
            <AlertDescription>
              {preview.localProfiles.map(([, name]) => name).join(", ")}. The resources set up on
              this computer are kept.
            </AlertDescription>
          </Alert>
        )}
      </div>

      <fieldset className="grid gap-2">
        <legend className="mb-1 text-sm font-medium">Profiles</legend>
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
                {profile.actions} actions · {profile.workflows} workflows · {profile.projects}{" "}
                projects · {profile.layouts} layouts
              </span>
            </label>
            {profile.conflict?.type === "sameId" && mode === "merge" && (
              <div className="flex flex-wrap items-center gap-2 pl-6 text-sm">
                <Badge variant="outline">Already here as “{profile.conflict.name}”</Badge>
                <Select
                  value={conflicts[profile.id] ?? "keepBoth"}
                  onValueChange={(value) => {
                    setConflicts({ ...conflicts, [profile.id]: value as ConflictResolution })
                  }}
                >
                  <SelectTrigger
                    className="h-8 w-56"
                    aria-label={`What to do with ${profile.name}`}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {RESOLUTIONS.map((r) => (
                      <SelectItem key={r.value} value={r.value}>
                        {r.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}
            {profile.conflict?.type === "sameName" && mode === "merge" && (
              <p className="pl-6 text-xs text-muted-foreground">
                A different local profile has the same name; both are kept.
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
            Apply module settings
          </label>
          <ul className="grid gap-1 pl-6 text-sm">
            {preview.modules.map((module) => (
              <li key={module.id} className="flex flex-wrap items-center gap-2">
                <span>{module.name ?? module.id}</span>
                <Badge variant={module.status === "ready" ? "secondary" : "outline"}>
                  {module.status === "ready"
                    ? module.enabled
                      ? "enabled"
                      : "disabled"
                    : module.status === "missing"
                      ? "not installed"
                      : "incompatible"}
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
          Apply application settings (updates)
        </label>
      )}

      {preview.unknownActionTypes.length > 0 && (
        <Alert>
          <TriangleAlert />
          <AlertTitle>Some actions need modules this DeckPilot does not have</AlertTitle>
          <AlertDescription>
            {preview.unknownActionTypes.join(", ")}. They are imported and shown as unavailable.
          </AlertDescription>
        </Alert>
      )}
      {preview.missingResources.length > 0 && (
        <Alert>
          <TriangleAlert />
          <AlertTitle>Resources to set up on this computer</AlertTitle>
          <AlertDescription>
            {preview.missingResources.map((r) => r.key).join(", ")}. You can map them after the
            import.
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

function ImportDone({
  summary,
  preview,
  onClose,
}: {
  summary: ImportSummary
  preview: ImportPreview
  onClose: () => void
}) {
  const missing = preview.missingResources
  return (
    <ModalShell
      size="medium"
      icon={FileCheck2}
      title="Import complete"
      description={`${summary.imported.length} profile(s) imported${
        summary.removed.length > 0 ? `, ${summary.removed.length} replaced` : ""
      }${summary.skipped.length > 0 ? `, ${summary.skipped.length} skipped` : ""}.`}
      onClose={onClose}
      secondary={
        missing.length > 0
          ? {
              label: "Map resources now",
              onClick: () => {
                void emitTo("main", NAVIGATE_EVENT, "projects/projects").then(() =>
                  api.showMainWindow(),
                )
                onClose()
              },
            }
          : undefined
      }
      primary={{ label: "Done", onClick: onClose }}
    >
      {summary.renamed.length > 0 && (
        <p className="text-sm">Imported as copies: {summary.renamed.join(", ")}.</p>
      )}
      {summary.settings.length > 0 && (
        <p className="text-sm">Application settings applied: {summary.settings.join(", ")}.</p>
      )}
      {missing.length > 0 && (
        <div className="grid gap-2">
          <h2 className="text-sm font-medium">Set up these resources on this computer</h2>
          <ul className="grid gap-1 text-sm">
            {missing.map((resource) => (
              <li key={resource.key}>
                <span className="font-mono">{resource.key}</span>
                {resource.kind && (
                  <span className="text-muted-foreground">
                    {" "}
                    · {RESOURCE_KIND_LABELS[resource.kind]}
                  </span>
                )}
                <span className="text-xs text-muted-foreground">
                  {" "}
                  — used by {resource.profiles.join(", ")}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </ModalShell>
  )
}
