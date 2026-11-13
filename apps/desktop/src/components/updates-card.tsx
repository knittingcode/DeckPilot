import { Download, RefreshCw, RotateCw } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ErrorNotice } from "@/components/page"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Label } from "@/components/ui/label"
import { Progress } from "@/components/ui/progress"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { api, errorMessage } from "@/lib/api"
import { useTitlebar } from "@/hooks/use-titlebar"
import { useAppContext } from "@/lib/app-context"
import { formatTimestamp } from "@/lib/format"
import type { UpdateChannel, UpdateMode, UpdatePreferences } from "@/lib/types"

const MODES: { value: UpdateMode; label: string; description: string }[] = [
  { value: "notifyOnly", label: "Notify me", description: "Show that an update is available." },
  {
    value: "downloadAndAsk",
    label: "Download, then ask",
    description: "Download in the background and install after you confirm.",
  },
  {
    value: "installWhenIdle",
    label: "Install when idle",
    description:
      "Install automatically after ten minutes without device input. Not recommended until updates have been tested on your system.",
  },
]

const CHANNELS: { value: UpdateChannel; label: string; description: string }[] = [
  { value: "stable", label: "Stable", description: "Only stable releases." },
  {
    value: "prerelease",
    label: "Pre-releases",
    description:
      "Also alpha, beta and release-candidate builds, which may have unfinished features or bugs. You move on to the stable release once it is newer; switching back to Stable never installs an older version.",
  },
]

const INTERVALS = [6, 12, 24, 72, 168]

export function UpdatesCard() {
  const { updates, setUpdates } = useAppContext()
  const [busy, setBusy] = useState(false)
  const checkNow = () => {
    void run(() => api.checkForUpdates())
  }
  const downloading = updates?.phase === "downloading" ? updates.progress : null
  useTitlebar({
    actions: [
      {
        id: "updates.check",
        label: "Check for updates",
        icon: RefreshCw,
        placement: "primary",
        busy: updates?.phase === "checking",
        disabled: !updates?.configured || busy || updates.phase !== "idle",
        hint: updates?.configured ? undefined : "Updates are not available in this build",
        onSelect: checkNow,
      },
    ],
    progress:
      downloading === null
        ? undefined
        : downloading.total
          ? Math.round((downloading.downloaded / downloading.total) * 100)
          : true,
  })
  if (!updates) return null
  const { preferences, available, progress, phase } = updates

  const savePreferences = (change: Partial<UpdatePreferences>) => {
    api
      .setUpdatePreferences({ ...preferences, ...change })
      .then(setUpdates, (e: unknown) => toast.error(errorMessage(e)))
  }
  const run = async (operation: () => Promise<unknown>) => {
    setBusy(true)
    try {
      await operation()
      setUpdates(await api.updateState())
    } catch (e) {
      toast.error(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }
  const percent =
    progress?.total && progress.total > 0
      ? Math.round((progress.downloaded / progress.total) * 100)
      : null

  return (
    <Card>
      <CardHeader>
        <CardTitle>Updates</CardTitle>
        <CardDescription>
          Version {updates.currentVersion} ({updates.target}). Updates come from GitHub Releases and
          are installed only if their signature verifies. Your configuration is never uploaded.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-4">
        {!updates.configured && (
          <Alert>
            <AlertTitle>Updates are not available in this build</AlertTitle>
            <AlertDescription>
              Development and self-built versions have no update signing key. Official releases
              check for updates automatically.
            </AlertDescription>
          </Alert>
        )}
        {updates.error && <ErrorNotice error={updates.error} />}

        <p className="text-xs text-muted-foreground">
          {phase === "checking"
            ? "Checking for updates…"
            : preferences.lastCheck
              ? `Last checked ${formatTimestamp(preferences.lastCheck)}`
              : "Not checked yet"}
        </p>

        {available ? (
          <div className="grid gap-3 rounded-lg border p-3">
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-medium">Version {available.version} is available</span>
              {available.date && (
                <span className="text-xs text-muted-foreground">{available.date}</span>
              )}
            </div>
            {available.notes && (
              <pre className="max-h-48 overflow-auto rounded-md bg-muted p-2 text-xs whitespace-pre-wrap">
                {available.notes}
              </pre>
            )}
            {phase === "downloading" && (
              <div className="grid gap-1">
                <Progress value={percent ?? undefined} aria-label="Download progress" />
                <span className="text-xs text-muted-foreground">
                  {percent !== null ? `${percent}%` : "Downloading…"}
                </span>
              </div>
            )}
            <div className="flex flex-wrap gap-2">
              {phase !== "ready" && (
                <Button
                  disabled={busy || phase !== "idle"}
                  onClick={() => {
                    // Progress follows in its own window as well.
                    void api.openWindow("updates.progress").catch(() => undefined)
                    void run(() => api.downloadUpdate())
                  }}
                >
                  <Download /> Download
                </Button>
              )}
              {phase === "ready" && (
                <Button
                  disabled={busy}
                  onClick={() => {
                    void api.openWindow("updates.progress").catch(() => undefined)
                    void run(() => api.installUpdate())
                  }}
                >
                  <RotateCw /> Install and restart
                </Button>
              )}
            </div>
            <p className="text-xs text-muted-foreground">
              Installing stops the background agent after its current task, releases the devices,
              installs, and restarts DeckPilot with the new agent.
            </p>
          </div>
        ) : (
          updates.configured && (
            <p className="text-sm text-muted-foreground">DeckPilot is up to date.</p>
          )
        )}

        <div className="grid gap-4 sm:grid-cols-2">
          <div className="grid gap-2">
            <Label htmlFor="update-mode">When an update is available</Label>
            <Select
              value={preferences.mode}
              onValueChange={(mode) => {
                savePreferences({ mode: mode as UpdateMode })
              }}
            >
              <SelectTrigger id="update-mode" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {MODES.map((mode) => (
                  <SelectItem key={mode.value} value={mode.value}>
                    {mode.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              {MODES.find((m) => m.value === preferences.mode)?.description}
            </p>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="update-channel">Update channel</Label>
            <Select
              value={preferences.channel}
              disabled={busy || (phase !== "idle" && phase !== "ready")}
              onValueChange={(channel) => {
                savePreferences({ channel: channel as UpdateChannel })
              }}
            >
              <SelectTrigger id="update-channel" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {CHANNELS.map((channel) => (
                  <SelectItem key={channel.value} value={channel.value}>
                    {channel.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              {CHANNELS.find((c) => c.value === preferences.channel)?.description}
            </p>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="update-interval">Check every</Label>
            <Select
              value={String(preferences.checkIntervalHours)}
              disabled={!preferences.automaticChecks}
              onValueChange={(hours) => {
                savePreferences({ checkIntervalHours: Number(hours) })
              }}
            >
              <SelectTrigger id="update-interval" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {[...new Set([...INTERVALS, preferences.checkIntervalHours])]
                  .sort((a, b) => a - b)
                  .map((hours) => (
                    <SelectItem key={hours} value={String(hours)}>
                      {hours < 24
                        ? `${hours} hours`
                        : `${hours / 24} day${hours === 24 ? "" : "s"}`}
                    </SelectItem>
                  ))}
              </SelectContent>
            </Select>
            <label className="flex items-center gap-2 text-sm">
              <Switch
                checked={preferences.automaticChecks}
                onCheckedChange={(automaticChecks) => {
                  savePreferences({ automaticChecks })
                }}
              />
              Check automatically while DeckPilot is open
            </label>
          </div>
        </div>
      </CardContent>
    </Card>
  )
}
