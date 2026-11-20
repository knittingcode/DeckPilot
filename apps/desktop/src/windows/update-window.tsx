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
import { type TranslationKey, i18n, useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"

import { useWindow } from "./window-context"

const PHASES: Record<Exclude<UpdateState["phase"], "idle">, TranslationKey> = {
  checking: "settings:updates.checking",
  downloading: "settings:updates.downloading",
  ready: "settings:updateWindow.ready",
  installing: "settings:updateWindow.installing",
}

/**
 * Progress of an update: download, signature verification and
 * installation. Follows the backend's update state; actions are the same
 * as in Settings. Closing the window does not cancel anything. Builds that
 * do not update themselves (package managers, stores, no key) only explain
 * how they are updated; the window offers nothing else.
 */
export function UpdateWindow() {
  const { t } = useTranslation()
  const format = useFormat()
  const { close } = useWindow()
  const [state, setState] = useState<UpdateState | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    api.updateState().then(setState, (e: unknown) => {
      setError(failureMessage(i18n.t("settings:updateWindow.statusUnreadable"), e))
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
    ? { label: t("settings:shared.close"), onClick: close }
    : !available
      ? {
          label: t("settings:updateWindow.checkNow"),
          loadingLabel: t("settings:updateWindow.checkingShort"),
          loading: busy || phase === "checking",
          onClick: () => {
            run(
              () => api.checkForUpdates(),
              t("settings:updates.checkFailed"),
              t("settings:updates.checkInternet"),
            )
          },
        }
      : phase === "ready" || phase === "installing"
        ? {
            label: t("settings:updates.installAndRestart"),
            loadingLabel: t("settings:updateWindow.installing"),
            loading: busy || phase === "installing",
            onClick: () => {
              run(() => api.installUpdate(), t("settings:updates.notInstalled"))
            },
          }
        : {
            label: t("settings:updates.download"),
            loadingLabel: t("settings:updates.downloading"),
            loading: busy || phase === "downloading",
            onClick: () => {
              run(
                () => api.downloadUpdate(),
                t("settings:updates.notDownloaded"),
                t("settings:updates.checkInternet"),
              )
            },
          }
  return (
    <ModalShell
      size="compact"
      icon={phase === "ready" ? RotateCw : available ? Download : RefreshCw}
      title={
        selfUpdate && available
          ? t("settings:updateWindow.titleVersion", { version: available.version })
          : t("settings:updateWindow.title")
      }
      description={
        state
          ? selfUpdate
            ? t("settings:updateWindow.installed", { version: state.currentVersion })
            : state.updateProvider === "none"
              ? t("settings:updateWindow.installedNoUpdates", { version: state.currentVersion })
              : t("settings:updateWindow.installedFrom", {
                  version: state.currentVersion,
                  channel: t(CHANNEL_LABELS[state.distributionChannel]),
                })
          : undefined
      }
      loading={!state && !error}
      error={error}
      onClose={close}
      secondary={selfUpdate ? { label: t("settings:shared.close"), onClick: close } : undefined}
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
          {t("settings:updateWindow.versionNotice", { version: available.version })}
        </p>
      )}
      {state?.error && <UpdateErrorNotice error={state.error} />}
      {selfUpdate && !available && phase === "idle" && (
        <p className="text-sm text-muted-foreground">{t("settings:updates.upToDate")}</p>
      )}
      {selfUpdate && phase !== "idle" && (
        <p className="text-sm text-muted-foreground">{t(PHASES[phase])}</p>
      )}
      {selfUpdate && phase === "downloading" && (
        <div className="grid gap-1">
          <Progress
            value={percent ?? undefined}
            aria-label={t("settings:updates.downloadProgress")}
          />
          <span className="text-xs text-muted-foreground">
            {percent !== null
              ? format.percent(percent / 100)
              : t("settings:updateWindow.sizeUnknown")}
          </span>
        </div>
      )}
      {selfUpdate && available?.notes && (
        <pre className="relative max-h-48 overflow-auto rounded-md bg-muted p-2 text-xs whitespace-pre-wrap">
          {available.notes}
        </pre>
      )}
      {selfUpdate && (
        <p className="text-xs text-muted-foreground">{t("settings:updateWindow.signature")}</p>
      )}
    </ModalShell>
  )
}
