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
import { type TranslationKey, useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"

const MODES: { value: UpdateMode; label: TranslationKey; description: TranslationKey }[] = [
  {
    value: "notifyOnly",
    label: "settings:updates.mode.notifyOnly",
    description: "settings:updates.mode.notifyOnlyDescription",
  },
  {
    value: "downloadAndAsk",
    label: "settings:updates.mode.downloadAndAsk",
    description: "settings:updates.mode.downloadAndAskDescription",
  },
  {
    value: "installWhenIdle",
    label: "settings:updates.mode.installWhenIdle",
    description: "settings:updates.mode.installWhenIdleDescription",
  },
]

const CHANNELS: { value: UpdateChannel; label: TranslationKey; description: TranslationKey }[] = [
  {
    value: "stable",
    label: "settings:updates.channel.stable",
    description: "settings:updates.channel.stableDescription",
  },
  {
    value: "prerelease",
    label: "settings:updates.channel.prerelease",
    description: "settings:updates.channel.prereleaseDescription",
  },
]

const INTERVALS = [6, 12, 24, 72, 168]

export function UpdatesCard() {
  const { t } = useTranslation()
  const format = useFormat()
  const { updates, setUpdates } = useAppContext()
  const [busy, setBusy] = useState(false)
  const internet = t("settings:updates.checkInternet")
  const checkNow = () => {
    void run(() => api.checkForUpdates(), t("settings:updates.checkFailed"), internet)
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
            label: t("settings:updates.check"),
            icon: RefreshCw,
            placement: "primary",
            busy: updates?.phase === "checking",
            disabled: !controls.checkEnabled || busy || updates?.phase !== "idle",
            hint: controls.checkEnabled ? undefined : t("settings:updates.notAvailable"),
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
  const modeDescription = MODES.find((m) => m.value === preferences.mode)?.description
  const channelDescription = CHANNELS.find((c) => c.value === preferences.channel)?.description

  const savePreferences = (change: Partial<UpdatePreferences>) => {
    api.setUpdatePreferences({ ...preferences, ...change }).then(setUpdates, (e: unknown) => {
      toastFailure(t("settings:updates.preferencesNotSaved"), e)
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
      ? t("settings:updates.checking")
      : preferences.lastCheck
        ? t("settings:updates.lastChecked", { time: formatTimestamp(preferences.lastCheck) })
        : t("settings:updates.notChecked")

  return (
    <>
      <p className="text-sm text-muted-foreground">
        {controls.selfUpdate
          ? t("settings:updates.introSelfUpdate")
          : controls.managed || controls.command
            ? t("settings:updates.introExternal", { deliveredBy: deliveredBy(updates) })
            : t("settings:updates.introNeverUploaded")}
      </p>

      {updates.developmentOverride && <DevelopmentOverrideNotice state={updates} />}
      {updates.updateProvider === "none" && (
        <Alert>
          <AlertTitle>{t("settings:updates.notAvailable")}</AlertTitle>
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
        <SettingsSection title={t("settings:agent.status")}>
          <SettingsGroup>
            <SettingsRow
              label={
                available && (controls.selfUpdate || controls.versionNotice)
                  ? t("settings:updates.versionAvailable", { version: available.version })
                  : controls.selfUpdate
                    ? t("settings:updates.upToDate")
                    : t("settings:updates.versionValue", { version: updates.currentVersion })
              }
              description={
                <>
                  {controls.versionNotice && available && (
                    <span className="block">{t("settings:updates.usePackageManager")}</span>
                  )}
                  {controls.selfUpdate && available?.date && (
                    <span className="block">
                      {t("settings:updates.released", { date: available.date })}
                    </span>
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
                    void run(
                      () => api.downloadUpdate(),
                      t("settings:updates.notDownloaded"),
                      internet,
                    )
                  }}
                >
                  <Download /> {t("settings:updates.download")}
                </Button>
              )}
              {controls.selfUpdate && available && phase === "ready" && (
                <Button
                  size="sm"
                  disabled={busy}
                  onClick={() => {
                    void api.openWindow("updates.progress").catch(() => undefined)
                    void run(() => api.installUpdate(), t("settings:updates.notInstalled"))
                  }}
                >
                  <RotateCw /> {t("settings:updates.installAndRestart")}
                </Button>
              )}
            </SettingsRow>
            {controls.selfUpdate && available && (
              <div className="grid gap-2 py-3">
                {phase === "downloading" && (
                  <div className="grid gap-1">
                    <Progress
                      value={percent ?? undefined}
                      aria-label={t("settings:updates.downloadProgress")}
                    />
                    <span className="text-caption text-muted-foreground">
                      {percent !== null
                        ? format.percent(percent / 100)
                        : t("settings:updates.downloading")}
                    </span>
                  </div>
                )}
                {available.notes && (
                  <pre className="relative max-h-48 overflow-auto rounded-md bg-muted p-2 text-xs whitespace-pre-wrap select-text">
                    {available.notes}
                  </pre>
                )}
                <p className="text-caption text-muted-foreground">
                  {phase === "ready"
                    ? t("settings:updates.readyInstallExplanation")
                    : t("settings:updates.installExplanation")}
                </p>
              </div>
            )}
          </SettingsGroup>
        </SettingsSection>
      )}

      {(controls.mode || controls.channelChoice || controls.schedule) && (
        <SettingsSection title={t("settings:updates.preferences")}>
          <SettingsGroup>
            {controls.mode && (
              <SettingsRow
                label={t("settings:updates.modeLabel")}
                htmlFor="update-mode"
                description={modeDescription && t(modeDescription)}
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
                        {t(mode.label)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </SettingsRow>
            )}
            {controls.channelChoice && (
              <SettingsRow
                label={t("settings:updates.channelLabel")}
                htmlFor="update-channel"
                description={channelDescription && t(channelDescription)}
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
                        {t(channel.label)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </SettingsRow>
            )}
            {controls.schedule && (
              <>
                <SettingsRow
                  label={t("settings:updates.automatic")}
                  htmlFor="update-automatic"
                  description={t("settings:updates.automaticDescription")}
                >
                  <Switch
                    id="update-automatic"
                    checked={preferences.automaticChecks}
                    onCheckedChange={(automaticChecks) => {
                      savePreferences({ automaticChecks })
                    }}
                  />
                </SettingsRow>
                <SettingsRow label={t("settings:updates.interval")} htmlFor="update-interval">
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
                              ? t("settings:updates.hours", { count: hours })
                              : t("settings:updates.days", { count: hours / 24 })}
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

      <SettingsSection title={t("settings:updates.installation")}>
        <InstallationDetails state={updates} />
      </SettingsSection>
    </>
  )
}

/** Version, channels and who updates this installation. */
export function InstallationDetails({ state }: { state: UpdateState }) {
  const { t } = useTranslation()
  const rows: [string, string][] = [
    [t("settings:about.version"), state.currentVersion],
    [t("settings:updates.releaseChannel"), t(RELEASE_CHANNEL_LABELS[state.releaseChannel])],
    [t("settings:updates.installedFrom"), installedFrom(state)],
    [t("settings:pages.updates.title"), providerLabel(state)],
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
  const { t } = useTranslation()
  return (
    <Alert>
      <FlaskConical />
      <AlertTitle className="flex flex-wrap items-center gap-2">
        <Badge variant="outline">{t("settings:updates.override.badge")}</Badge>
        {t("settings:updates.override.title")}
      </AlertTitle>
      <AlertDescription>
        {t("settings:updates.override.description", {
          channel: t(CHANNEL_LABELS[state.distributionChannel]),
          variable: "DECKPILOT_DISTRIBUTION_OVERRIDE",
        })}
      </AlertDescription>
    </Alert>
  )
}

/** Store builds: the store installs updates; there is nothing to check or install here. */
export function ManagedNotice({ state, instruction }: { state: UpdateState; instruction: string }) {
  const { t } = useTranslation()
  return (
    <Alert>
      <PackageCheck />
      <AlertTitle>{deliveredBy(state)}</AlertTitle>
      <AlertDescription>
        <p>
          <InlineCode text={sentenceOf(instruction)} />
        </p>
        <p>{t("settings:updates.restartAfterExternal")}</p>
      </AlertDescription>
    </Alert>
  )
}

/** Package-manager builds: the documented command, copyable; never an install button. */
export function PackageManagerNotice({ state, command }: { state: UpdateState; command: string }) {
  const { t } = useTranslation()
  return (
    <Alert>
      <PackageCheck />
      <AlertTitle>{deliveredBy(state)}</AlertTitle>
      <AlertDescription className="grid gap-2">
        <p>{t("settings:updates.packageManager")}</p>
        <div className="flex w-full items-center gap-2">
          <code
            className="relative flex-1 overflow-x-auto rounded-md bg-muted px-2 py-1 font-mono text-xs whitespace-nowrap text-foreground"
            aria-label={t("settings:updates.command")}
          >
            {command}
          </code>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              navigator.clipboard.writeText(command).then(
                () => toast.success(t("settings:updates.commandCopied")),
                (e: unknown) => {
                  toastFailure(
                    t("settings:updates.commandNotCopied"),
                    e,
                    t("settings:updates.copyByHand"),
                  )
                },
              )
            }}
          >
            <ClipboardCopy /> {t("settings:shared.copy")}
          </Button>
        </div>
        <p>{t("settings:updates.restartAfterExternal")}</p>
      </AlertDescription>
    </Alert>
  )
}

/** An update error of the backend, with a title that says what happened. */
export function UpdateErrorNotice({ error }: { error: string }) {
  const { t } = useTranslation()
  const described = describeUpdateError(error)
  return (
    <Alert variant="destructive">
      <TriangleAlert />
      <AlertTitle>{described.title}</AlertTitle>
      <AlertDescription>
        <p>{described.message}</p>
        {described.detail && (
          <p className="text-xs opacity-80">
            {t("settings:updates.details", { detail: described.detail })}
          </p>
        )}
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
