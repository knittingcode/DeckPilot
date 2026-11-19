import {
  ClipboardCopy,
  Download,
  FlaskConical,
  PackageCheck,
  RefreshCw,
  RotateCw,
  TriangleAlert,
} from "lucide-react"
import { Fragment, useState } from "react"
import { toast } from "sonner"

import {
  Property,
  PropertyList,
  SettingsGroup,
  SettingsRow,
  SettingsSection,
} from "@/components/settings-layout"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { useTitlebar } from "@/hooks/use-titlebar"
import { useAppContext } from "@/lib/app-context"
import { formatTimestamp } from "@/lib/format"
import type { UpdateChannel, UpdateMode, UpdatePreferences, UpdateState } from "@/lib/types"
import {
  CHANNEL_LABELS,
  RELEASE_CHANNEL_LABELS,
  deliveredBy,
  describeUpdateError,
  installedFrom,
  notConfiguredExplanation,
  providerLabel,
  updateControls,
} from "@/lib/update-distribution"

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

const INTERNET = "Check your internet connection and try again"

export function UpdatesCard() {
  const { updates, setUpdates } = useAppContext()
  const [busy, setBusy] = useState(false)
  const checkNow = () => {
    void run(() => api.checkForUpdates(), "Checking for updates failed", INTERNET)
  }
  const controls = updates ? updateControls(updates) : null
  const downloading =
    controls?.selfUpdate && updates?.phase === "downloading" ? updates.progress : null
  useTitlebar({
    // Store builds have no check at all; the store updates them.
    actions: controls?.check
      ? [
          {
            id: "updates.check",
            label: "Check for updates",
            icon: RefreshCw,
            placement: "primary",
            busy: updates?.phase === "checking",
            disabled: !controls.checkEnabled || busy || updates?.phase !== "idle",
            hint: controls.checkEnabled ? undefined : "Updates are not available in this build",
            onSelect: checkNow,
          },
        ]
      : [],
    progress:
      downloading === null
        ? undefined
        : downloading.total
          ? Math.round((downloading.downloaded / downloading.total) * 100)
          : true,
  })
  if (!updates || !controls) return null
  const { preferences, available, progress, phase } = updates

  const savePreferences = (change: Partial<UpdatePreferences>) => {
    api.setUpdatePreferences({ ...preferences, ...change }).then(setUpdates, (e: unknown) => {
      toastFailure("The update settings were not saved", e)
    })
  }
  const run = async (operation: () => Promise<unknown>, failed: string, next?: string) => {
    setBusy(true)
    try {
      await operation()
      setUpdates(await api.updateState())
    } catch (e) {
      toastFailure(failed, e, next)
    } finally {
      setBusy(false)
    }
  }
  const percent =
    progress?.total && progress.total > 0
      ? Math.round((progress.downloaded / progress.total) * 100)
      : null

  const lastCheck =
    phase === "checking"
      ? "Checking for updates…"
      : preferences.lastCheck
        ? `Last checked ${formatTimestamp(preferences.lastCheck)}`
        : "Not checked yet"

  return (
    <>
      <p className="text-sm text-muted-foreground">
        {controls.selfUpdate
          ? "Updates come from GitHub Releases and are installed only if their signature verifies. Your configuration is never uploaded."
          : controls.managed || controls.command
            ? `${deliveredBy(updates)}. DeckPilot does not download or install updates itself in this installation. Your configuration is never uploaded.`
            : "Your configuration is never uploaded."}
      </p>

      {updates.developmentOverride && <DevelopmentOverrideNotice state={updates} />}
      {updates.updateProvider === "none" && (
        <Alert>
          <AlertTitle>Updates are not available in this build</AlertTitle>
          <AlertDescription>
            {notConfiguredExplanation(updates.notConfiguredReason)}
          </AlertDescription>
        </Alert>
      )}
      {controls.managed && <ManagedNotice state={updates} instruction={controls.managed} />}
      {controls.command && <PackageManagerNotice state={updates} command={controls.command} />}
      {updates.error && <UpdateErrorNotice error={updates.error} />}

      {(controls.selfUpdate ||
        controls.versionNotice ||
        (controls.check && controls.checkEnabled)) && (
        <SettingsSection title="Status">
          <SettingsGroup>
            <SettingsRow
              label={
                available && (controls.selfUpdate || controls.versionNotice)
                  ? `Version ${available.version} is available`
                  : controls.selfUpdate
                    ? "DeckPilot is up to date."
                    : `Version ${updates.currentVersion}`
              }
              description={
                <>
                  {controls.versionNotice && available && (
                    <span className="block">Update with your package manager.</span>
                  )}
                  {controls.selfUpdate && available?.date && (
                    <span className="block">Released {available.date}</span>
                  )}
                  {controls.check && controls.checkEnabled && (
                    <span className="block">{lastCheck}</span>
                  )}
                </>
              }
            >
              {controls.selfUpdate && available && phase !== "ready" && (
                <Button
                  size="sm"
                  disabled={busy || phase !== "idle"}
                  onClick={() => {
                    // Progress follows in its own window as well.
                    void api.openWindow("updates.progress").catch(() => undefined)
                    void run(() => api.downloadUpdate(), "The update was not downloaded", INTERNET)
                  }}
                >
                  <Download /> Download
                </Button>
              )}
              {controls.selfUpdate && available && phase === "ready" && (
                <Button
                  size="sm"
                  disabled={busy}
                  onClick={() => {
                    void api.openWindow("updates.progress").catch(() => undefined)
                    void run(() => api.installUpdate(), "The update was not installed")
                  }}
                >
                  <RotateCw /> Install and restart
                </Button>
              )}
            </SettingsRow>
            {controls.selfUpdate && available && (
              <div className="grid gap-2 py-3">
                {phase === "downloading" && (
                  <div className="grid gap-1">
                    <Progress value={percent ?? undefined} aria-label="Download progress" />
                    <span className="text-caption text-muted-foreground">
                      {percent !== null ? `${percent}%` : "Downloading…"}
                    </span>
                  </div>
                )}
                {available.notes && (
                  <pre className="max-h-48 overflow-auto rounded-md bg-muted p-2 text-xs whitespace-pre-wrap select-text">
                    {available.notes}
                  </pre>
                )}
                <p className="text-caption text-muted-foreground">
                  {phase === "ready" ? "Downloaded and verified. " : ""}Installing stops the
                  background agent after its current task, releases the devices, installs, and
                  restarts DeckPilot with the new agent.
                </p>
              </div>
            )}
          </SettingsGroup>
        </SettingsSection>
      )}

      {(controls.mode || controls.channelChoice || controls.schedule) && (
        <SettingsSection title="Preferences">
          <SettingsGroup>
            {controls.mode && (
              <SettingsRow
                label="When an update is available"
                htmlFor="update-mode"
                description={MODES.find((m) => m.value === preferences.mode)?.description}
              >
                <Select
                  value={preferences.mode}
                  onValueChange={(mode) => {
                    savePreferences({ mode: mode as UpdateMode })
                  }}
                >
                  <SelectTrigger id="update-mode" size="sm" className="w-52">
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
              </SettingsRow>
            )}
            {controls.channelChoice && (
              <SettingsRow
                label="Update channel"
                htmlFor="update-channel"
                description={CHANNELS.find((c) => c.value === preferences.channel)?.description}
              >
                <Select
                  value={preferences.channel}
                  disabled={busy || (phase !== "idle" && phase !== "ready")}
                  onValueChange={(channel) => {
                    savePreferences({ channel: channel as UpdateChannel })
                  }}
                >
                  <SelectTrigger id="update-channel" size="sm" className="w-52">
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
              </SettingsRow>
            )}
            {controls.schedule && (
              <>
                <SettingsRow
                  label="Check automatically"
                  htmlFor="update-automatic"
                  description="While DeckPilot is open."
                >
                  <Switch
                    id="update-automatic"
                    checked={preferences.automaticChecks}
                    onCheckedChange={(automaticChecks) => {
                      savePreferences({ automaticChecks })
                    }}
                  />
                </SettingsRow>
                <SettingsRow label="Check every" htmlFor="update-interval">
                  <Select
                    value={String(preferences.checkIntervalHours)}
                    disabled={!preferences.automaticChecks}
                    onValueChange={(hours) => {
                      savePreferences({ checkIntervalHours: Number(hours) })
                    }}
                  >
                    <SelectTrigger id="update-interval" size="sm" className="w-52">
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
                </SettingsRow>
              </>
            )}
          </SettingsGroup>
        </SettingsSection>
      )}

      <SettingsSection title="This installation">
        <InstallationDetails state={updates} />
      </SettingsSection>
    </>
  )
}

/** Version, channels and who updates this installation. */
export function InstallationDetails({ state }: { state: UpdateState }) {
  const rows: [string, string][] = [
    ["Version", state.currentVersion],
    ["Release channel", RELEASE_CHANNEL_LABELS[state.releaseChannel]],
    ["Installed from", installedFrom(state)],
    ["Updates", providerLabel(state)],
  ]
  return (
    <PropertyList className="border-t pt-3">
      {rows.map(([term, value]) => (
        <Property key={term} label={term}>
          {value}
        </Property>
      ))}
    </PropertyList>
  )
}

/** A debug build pretending to be installed from another channel. */
export function DevelopmentOverrideNotice({ state }: { state: UpdateState }) {
  return (
    <Alert>
      <FlaskConical />
      <AlertTitle className="flex flex-wrap items-center gap-2">
        <Badge variant="outline">Development override</Badge>
        Simulated installation
      </AlertTitle>
      <AlertDescription>
        This debug build behaves as if installed from {CHANNEL_LABELS[state.distributionChannel]}{" "}
        (set by DECKPILOT_DISTRIBUTION_OVERRIDE). Release builds ignore the override.
      </AlertDescription>
    </Alert>
  )
}

/** Store builds: the store installs updates; there is nothing to check or install here. */
export function ManagedNotice({ state, instruction }: { state: UpdateState; instruction: string }) {
  return (
    <Alert>
      <PackageCheck />
      <AlertTitle>{deliveredBy(state)}</AlertTitle>
      <AlertDescription>
        <p>
          <InlineCode text={sentenceOf(instruction)} />
        </p>
        <p>{RESTART_AFTER_EXTERNAL_UPDATE}</p>
      </AlertDescription>
    </Alert>
  )
}

/** Package-manager builds: the documented command, copyable; never an install button. */
export function PackageManagerNotice({ state, command }: { state: UpdateState; command: string }) {
  return (
    <Alert>
      <PackageCheck />
      <AlertTitle>{deliveredBy(state)}</AlertTitle>
      <AlertDescription className="grid gap-2">
        <p>Update DeckPilot with your package manager:</p>
        <div className="flex w-full items-center gap-2">
          <code
            className="flex-1 overflow-x-auto rounded-md bg-muted px-2 py-1 font-mono text-xs whitespace-nowrap text-foreground"
            aria-label="Update command"
          >
            {command}
          </code>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              navigator.clipboard.writeText(command).then(
                () => toast.success("Command copied"),
                (e: unknown) => {
                  toastFailure("The command was not copied", e, "Select it and copy it by hand")
                },
              )
            }}
          >
            <ClipboardCopy /> Copy
          </Button>
        </div>
        <p>{RESTART_AFTER_EXTERNAL_UPDATE}</p>
      </AlertDescription>
    </Alert>
  )
}

const RESTART_AFTER_EXTERNAL_UPDATE =
  "After an update is installed, quit and reopen DeckPilot. If the background agent still runs the previous version, DeckPilot offers to restart it."

/** An update error of the backend, with a title that says what happened. */
export function UpdateErrorNotice({ error }: { error: string }) {
  const described = describeUpdateError(error)
  return (
    <Alert variant="destructive">
      <TriangleAlert />
      <AlertTitle>{described.title}</AlertTitle>
      <AlertDescription>
        <p>{described.message}</p>
        {described.detail && <p className="text-xs opacity-80">Details: {described.detail}</p>}
      </AlertDescription>
    </Alert>
  )
}

function sentenceOf(text: string): string {
  return /[.!?]$/.test(text) ? text : `${text}.`
}

/** `text` with `backticked` parts as code. */
function InlineCode({ text }: { text: string }) {
  return text.split("`").map((part, i) =>
    i % 2 === 1 ? (
      <code key={i} className="rounded bg-muted px-1 font-mono text-xs text-foreground">
        {part}
      </code>
    ) : (
      <Fragment key={i}>{part}</Fragment>
    ),
  )
}
