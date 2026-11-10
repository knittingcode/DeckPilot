import { listen } from "@tauri-apps/api/event"
import { Download, RefreshCw, RotateCw } from "lucide-react"
import { useEffect, useState } from "react"

import { ModalShell } from "@/components/modal-shell"
import { Progress } from "@/components/ui/progress"
import { UPDATE_STATE_EVENT, api, errorMessage } from "@/lib/api"
import type { UpdateState } from "@/lib/types"

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
 * as in Settings. Closing the window does not cancel anything.
 */
export function UpdateWindow() {
  const { close } = useWindow()
  const [state, setState] = useState<UpdateState | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    api.updateState().then(setState, (e: unknown) => {
      setError(errorMessage(e))
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
  const run = (operation: () => Promise<unknown>) => {
    setBusy(true)
    setError(null)
    operation()
      .then(() => api.updateState())
      .then(setState)
      .catch((e: unknown) => {
        setError(errorMessage(e))
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
  const primary = !state?.configured
    ? { label: "Close", onClick: close }
    : !available
      ? {
          label: "Check now",
          loadingLabel: "Checking…",
          loading: busy || phase === "checking",
          onClick: () => {
            run(() => api.checkForUpdates())
          },
        }
      : phase === "ready" || phase === "installing"
        ? {
            label: "Install and restart",
            loadingLabel: "Installing…",
            loading: busy || phase === "installing",
            onClick: () => {
              run(() => api.installUpdate())
            },
          }
        : {
            label: "Download",
            loadingLabel: "Downloading…",
            loading: busy || phase === "downloading",
            onClick: () => {
              run(() => api.downloadUpdate())
            },
          }
  return (
    <ModalShell
      size="compact"
      icon={phase === "ready" ? RotateCw : available ? Download : RefreshCw}
      title={available ? `DeckPilot ${available.version}` : "DeckPilot update"}
      description={
        state
          ? state.configured
            ? `Installed: ${state.currentVersion}`
            : "Updates are not available in this build: it has no update signing key."
          : undefined
      }
      loading={!state && !error}
      error={error ?? state?.error ?? null}
      onClose={close}
      secondary={state?.configured ? { label: "Close", onClick: close } : undefined}
      primary={primary}
    >
      {state?.configured && !available && phase === "idle" && (
        <p className="text-sm text-muted-foreground">DeckPilot is up to date.</p>
      )}
      {phase !== "idle" && <p className="text-sm text-muted-foreground">{PHASES[phase]}</p>}
      {phase === "downloading" && (
        <div className="grid gap-1">
          <Progress value={percent ?? undefined} aria-label="Download progress" />
          <span className="text-xs text-muted-foreground">
            {percent !== null ? `${percent}%` : "Size unknown"}
          </span>
        </div>
      )}
      {available?.notes && (
        <pre className="max-h-48 overflow-auto rounded-md bg-muted p-2 text-xs whitespace-pre-wrap">
          {available.notes}
        </pre>
      )}
      <p className="text-xs text-muted-foreground">
        Updates are installed only if their signature verifies. Your configuration stays encrypted
        and is never uploaded.
      </p>
    </ModalShell>
  )
}
