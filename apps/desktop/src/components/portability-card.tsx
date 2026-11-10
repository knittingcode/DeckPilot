import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog"
import { ArrowRightLeft, Download, TriangleAlert, Upload } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { isCancelled, useSensitive } from "@/lib/sensitive"
import { ErrorNotice } from "@/components/page"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { RESOURCE_KIND_LABELS } from "@/lib/format"
import type {
  ConflictResolution,
  ImportMode,
  ImportPreview,
  ImportSummary,
  MissingResource,
} from "@/lib/types"

const FILTERS = [{ name: "DeckPilot configuration", extensions: ["deckpilot"] }]

/** Encrypted export and import of the portable configuration. */
export function PortabilityCard() {
  const [exporting, setExporting] = useState(false)
  const [importPath, setImportPath] = useState<string | null>(null)
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ArrowRightLeft className="size-4" /> Export and import
        </CardTitle>
        <CardDescription>
          Move profiles, projects, actions, workflows, layouts and module settings to another
          computer in an encrypted file. Paths and other settings of this computer are not included.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          onClick={() => {
            setExporting(true)
          }}
        >
          <Download /> Export…
        </Button>
        <Button
          variant="outline"
          onClick={() => {
            openDialog({ multiple: false, directory: false, filters: FILTERS }).then(
              (path) => {
                if (typeof path === "string") setImportPath(path)
              },
              (e: unknown) => toast.error(errorMessage(e)),
            )
          }}
        >
          <Upload /> Import…
        </Button>
      </CardContent>
      <ExportDialog
        open={exporting}
        onClose={() => {
          setExporting(false)
        }}
      />
      {importPath && (
        <ImportDialog
          path={importPath}
          onClose={() => {
            setImportPath(null)
          }}
        />
      )}
    </Card>
  )
}

function ExportDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { profiles } = useAppContext()
  const sensitive = useSensitive()
  const [selected, setSelected] = useState<string[] | null>(null)
  const [includeModules, setIncludeModules] = useState(true)
  const [password, setPassword] = useState("")
  const [confirmation, setConfirmation] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const chosen = selected ?? profiles.profiles.map((p) => p.id)
  const ready = chosen.length > 0 && password.length >= 8 && password === confirmation

  const submit = async (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!ready) return
    setError(null)
    const path = await saveDialog({
      defaultPath: "deckpilot-configuration.deckpilot",
      filters: FILTERS,
    })
    if (!path) return
    setBusy(true)
    try {
      // Exports are sensitive: the user authenticates first.
      const result = await sensitive(
        () =>
          api.exportConfiguration({
            path,
            password,
            confirmation,
            profiles: chosen.length === profiles.profiles.length ? null : chosen,
            includeModuleSettings: includeModules,
          }),
        "export your configuration",
      )
      toast.success(`Exported ${result.profiles} profile(s)`, { description: result.path })
      setPassword("")
      setConfirmation("")
      onClose()
    } catch (e) {
      if (!isCancelled(e)) setError(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <form className="grid gap-4" onSubmit={(e) => void submit(e)}>
          <DialogHeader>
            <DialogTitle>Export configuration</DialogTitle>
            <DialogDescription>
              The file is encrypted with a password of its own. You need it to import the file.
            </DialogDescription>
          </DialogHeader>
          {error && <ErrorNotice error={error} />}
          <fieldset className="grid gap-2">
            <legend className="mb-1 text-sm font-medium">Profiles</legend>
            {profiles.profiles.map((profile) => (
              <label key={profile.id} className="flex items-center gap-2 text-sm">
                <Checkbox
                  checked={chosen.includes(profile.id)}
                  onCheckedChange={(checked) => {
                    setSelected(
                      checked === true
                        ? [...chosen, profile.id]
                        : chosen.filter((id) => id !== profile.id),
                    )
                  }}
                />
                {profile.name}
              </label>
            ))}
          </fieldset>
          <label className="flex items-center gap-2 text-sm">
            <Checkbox
              checked={includeModules}
              onCheckedChange={(checked) => {
                setIncludeModules(checked === true)
              }}
            />
            Include module settings
          </label>
          <div className="grid gap-2">
            <Label htmlFor="export-password">Export password</Label>
            <Input
              id="export-password"
              type="password"
              autoComplete="new-password"
              value={password}
              onChange={(e) => {
                setPassword(e.target.value)
              }}
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="export-password-confirmation">Repeat export password</Label>
            <Input
              id="export-password-confirmation"
              type="password"
              autoComplete="new-password"
              value={confirmation}
              onChange={(e) => {
                setConfirmation(e.target.value)
              }}
            />
          </div>
          {password.length > 0 && password.length < 8 && (
            <p className="text-sm text-destructive">Use at least 8 characters.</p>
          )}
          {confirmation.length > 0 && confirmation !== password && (
            <p className="text-sm text-destructive">The passwords do not match.</p>
          )}
          <DialogFooter>
            <Button type="submit" disabled={!ready || busy}>
              {busy ? "Encrypting…" : "Choose file and export"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

type ImportStep =
  | { step: "password" }
  | { step: "preview"; token: number; preview: ImportPreview }
  | { step: "done"; summary: ImportSummary; missing: MissingResource[] }

function ImportDialog({ path, onClose }: { path: string; onClose: () => void }) {
  const [state, setState] = useState<ImportStep>({ step: "password" })
  const close = () => {
    if (state.step === "preview") void api.cancelImport()
    onClose()
  }
  return (
    <Dialog
      open
      onOpenChange={(next) => {
        if (!next) close()
      }}
    >
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        {state.step === "password" && (
          <ImportPassword
            path={path}
            onPreview={(token, preview) => {
              setState({ step: "preview", token, preview })
            }}
          />
        )}
        {state.step === "preview" && (
          <ImportChoices
            token={state.token}
            preview={state.preview}
            onDone={(summary) => {
              setState({ step: "done", summary, missing: state.preview.missingResources })
            }}
          />
        )}
        {state.step === "done" && (
          <ImportDone summary={state.summary} missing={state.missing} onClose={onClose} />
        )}
      </DialogContent>
    </Dialog>
  )
}

function ImportPassword({
  path,
  onPreview,
}: {
  path: string
  onPreview: (token: number, preview: ImportPreview) => void
}) {
  const [password, setPassword] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const submit = (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    setBusy(true)
    setError(null)
    api
      .previewImport(path, password)
      .then((result) => {
        onPreview(result.token, result.preview)
      })
      .catch((e: unknown) => {
        setError(errorMessage(e))
      })
      .finally(() => {
        setBusy(false)
      })
  }
  return (
    <form className="grid gap-4" onSubmit={submit}>
      <DialogHeader>
        <DialogTitle>Import configuration</DialogTitle>
        <DialogDescription className="break-all">{path}</DialogDescription>
      </DialogHeader>
      {error && <ErrorNotice error={error} />}
      <div className="grid gap-2">
        <Label htmlFor="import-password">Export password</Label>
        <Input
          id="import-password"
          type="password"
          autoComplete="off"
          autoFocus
          value={password}
          onChange={(e) => {
            setPassword(e.target.value)
          }}
        />
      </div>
      <p className="text-xs text-muted-foreground">
        Nothing is changed yet: you see what the file contains first.
      </p>
      <DialogFooter>
        <Button type="submit" disabled={!password || busy}>
          {busy ? "Decrypting…" : "Show contents"}
        </Button>
      </DialogFooter>
    </form>
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
}: {
  token: number
  preview: ImportPreview
  onDone: (summary: ImportSummary) => void
}) {
  const { refresh, setModules } = useAppContext()
  const [mode, setMode] = useState<ImportMode>("merge")
  const [selected, setSelected] = useState(preview.profiles.map((p) => p.id))
  const [conflicts, setConflicts] = useState<Record<string, ConflictResolution>>({})
  const ready = preview.modules.some((m) => m.status === "ready")
  const [moduleSettings, setModuleSettings] = useState(ready)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const apply = () => {
    setBusy(true)
    setError(null)
    api
      .applyImport(token, { mode, profiles: selected, conflicts, moduleSettings })
      .then(async (summary) => {
        setModules(await api.listModules())
        refresh()
        onDone(summary)
      })
      .catch((e: unknown) => {
        setError(errorMessage(e))
      })
      .finally(() => {
        setBusy(false)
      })
  }

  return (
    <div className="grid gap-4">
      <DialogHeader>
        <DialogTitle>Review the import</DialogTitle>
        <DialogDescription>
          Exported by DeckPilot {preview.appVersion} on{" "}
          {new Date(preview.exportedAt * 1000).toLocaleString()}. Imported workflows do not run and
          imported profiles are not activated.
        </DialogDescription>
      </DialogHeader>
      {error && <ErrorNotice error={error} />}

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

      <DialogFooter>
        <Button onClick={apply} disabled={busy || (selected.length === 0 && !moduleSettings)}>
          {busy ? "Importing…" : "Import"}
        </Button>
      </DialogFooter>
    </div>
  )
}

function ImportDone({
  summary,
  missing,
  onClose,
}: {
  summary: ImportSummary
  missing: MissingResource[]
  onClose: () => void
}) {
  const { navigate, modules } = useAppContext()
  const projects = modules.some((m) => m.id === "projects" && m.enabled)
  return (
    <div className="grid gap-4">
      <DialogHeader>
        <DialogTitle>Import complete</DialogTitle>
        <DialogDescription>
          {summary.imported.length} profile(s) imported
          {summary.removed.length > 0 && `, ${summary.removed.length} replaced`}
          {summary.skipped.length > 0 && `, ${summary.skipped.length} skipped`}.
        </DialogDescription>
      </DialogHeader>
      {summary.renamed.length > 0 && (
        <p className="text-sm">Imported as copies: {summary.renamed.join(", ")}.</p>
      )}
      {missing.length > 0 && (
        <div className="grid gap-2">
          <h3 className="text-sm font-medium">Set up these resources on this computer</h3>
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
      <DialogFooter>
        {missing.length > 0 && projects && (
          <Button
            variant="outline"
            onClick={() => {
              onClose()
              navigate("projects/projects")
            }}
          >
            Map resources now
          </Button>
        )}
        <Button onClick={onClose}>Done</Button>
      </DialogFooter>
    </div>
  )
}
