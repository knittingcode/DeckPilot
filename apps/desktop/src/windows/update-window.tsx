import { listen } from "@tauri-apps/api/event"
import { Download, RefreshCw, RotateCw } from "lucide-react"
import { useEffect, useState } from "react"

import { ModalShell } from "@/components/modal-shell"
import { Progress } from "@/components/ui/progress"
import {
  DevelopmentOverrideNotice,
  ManagedNotice,
  PackageManagerNotice,
  UpdateErrorNotice,
} from "@/components/updates-card"
import { UPDATE_STATE_EVENT, api } from "@/lib/api"
import { failureMessage } from "@/lib/failure"
import type { UpdateState } from "@/lib/types"
import { CHANNEL_LABELS, notConfiguredExplanation, updateControls } from "@/lib/update-distribution"

import { useWindow } from "./window-context"

const PHASES: Record<UpdateState["phase"], string> = {
  idle: "",
  checking: "Checking for updates…",
  downloading: "Downloading…",
  ready: "Downloaded and verified. Installing restarts DeckPilot and its background agent.",
  installing: "Installing…",
}

/**
 * Progress of an update: download, signature verification and
 * installation. Follows the backend's update state; actions are the same
 * as in Settings. Closing the window does not cancel anything. Builds that
 * do not update themselves (package managers, stores, no key) only explain
 * how they are updated; the window offers nothing else.
 */
export function UpdateWindow() {
  const { close } = useWindow()
  const [state, setState] = useState<UpdateState | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    api.updateState().then(setState, (e: unknown) => {
      setError(failureMessage("The update status could not be read", e))
    })
    const stop = listen<UpdateState>(UPDATE_STATE_EVENT, (event) => {
      setState(event.payload)
    })
    return () => {
      void stop.then((unlisten) => {
        unlisten()
      })
    }
  }, [])
  const run = (operation: () => Promise<unknown>, failed: string, next?: string) => {
    setBusy(true)
    setError(null)
    operation()
      .then(() => api.updateState())
      .then(setState)
      .catch((e: unknown) => {
        setError(failureMessage(failed, e, next))
      })
      .finally(() => {
        setBusy(false)
      })
  }
  const available = state?.available
  const progress = state?.progress
  const percent =
    progress?.total && progress.total > 0
      ? Math.round((progress.downloaded / progress.total) * 100)
      : null
  const phase = state?.phase ?? "idle"
  const controls = state ? updateControls(state) : null
  const selfUpdate = controls?.selfUpdate ?? false
  const primary = !selfUpdate
    ? { label: "Close", onClick: close }
    : !available
      ? {
          label: "Check now",
          loadingLabel: "Checking…",
          loading: busy || phase === "checking",
          onClick: () => {
            run(
              () => api.checkForUpdates(),
              "Checking for updates failed",
              "Check your internet connection and try again",
            )
          },
        }
      : phase === "ready" || phase === "installing"
        ? {
            label: "Install and restart",
            loadingLabel: "Installing…",
            loading: busy || phase === "installing",
            onClick: () => {
              run(() => api.installUpdate(), "The update was not installed")
            },
          }
        : {
            label: "Download",
            loadingLabel: "Downloading…",
            loading: busy || phase === "downloading",
            onClick: () => {
              run(
                () => api.downloadUpdate(),
                "The update was not downloaded",
                "Check your internet connection and try again",
              )
            },
          }
  return (
    <ModalShell
      size="compact"
      icon={phase === "ready" ? RotateCw : available ? Download : RefreshCw}
      title={selfUpdate && available ? `DeckPilot ${available.version}` : "DeckPilot update"}
      description={
        state
          ? selfUpdate
            ? `Installed: ${state.currentVersion}`
            : state.updateProvider === "none"
              ? `Installed: ${state.currentVersion}. Updates are not available in this build.`
              : `Installed: ${state.currentVersion} from ${CHANNEL_LABELS[state.distributionChannel]}`
          : undefined
      }
      loading={!state && !error}
      error={error}
      onClose={close}
      secondary={selfUpdate ? { label: "Close", onClick: close } : undefined}
      primary={primary}
    >
      {state?.developmentOverride && <DevelopmentOverrideNotice state={state} />}
      {state?.updateProvider === "none" && (
        <p className="text-sm text-muted-foreground">
          {notConfiguredExplanation(state.notConfiguredReason)}
        </p>
      )}
      {state && controls?.managed && <ManagedNotice state={state} instruction={controls.managed} />}
      {state && controls?.command && (
        <PackageManagerNotice state={state} command={controls.command} />
      )}
      {controls?.versionNotice && available && (
        <p className="text-sm">
          Version {available.version} is available; update with your package manager.
        </p>
      )}
      {state?.error && <UpdateErrorNotice error={state.error} />}
      {selfUpdate && !available && phase === "idle" && (
        <p className="text-sm text-muted-foreground">DeckPilot is up to date.</p>
      )}
      {selfUpdate && phase !== "idle" && (
        <p className="text-sm text-muted-foreground">{PHASES[phase]}</p>
      )}
      {selfUpdate && phase === "downloading" && (
        <div className="grid gap-1">
          <Progress value={percent ?? undefined} aria-label="Download progress" />
          <span className="text-xs text-muted-foreground">
            {percent !== null ? `${percent}%` : "Size unknown"}
          </span>
        </div>
      )}
      {selfUpdate && available?.notes && (
        <pre className="max-h-48 overflow-auto rounded-md bg-muted p-2 text-xs whitespace-pre-wrap">
          {available.notes}
        </pre>
      )}
      {selfUpdate && (
        <p className="text-xs text-muted-foreground">
          Updates are installed only if their signature verifies. Your configuration stays encrypted
          and is never uploaded.
        </p>
      )}
    </ModalShell>
  )
}
