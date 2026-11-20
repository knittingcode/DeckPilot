import { Play, RefreshCw, RotateCw, TriangleAlert } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { InlineStatus } from "@/components/settings-layout"
import { Button } from "@/components/ui/button"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import { describeLayout } from "@/lib/format"
import type { AgentCapability, DeviceInfo } from "@/lib/types"
import { type TranslationKey, Trans, useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"

/** The agent's state as an inline status: running, stopped or being checked. */
export function AgentBadge() {
  const { t } = useTranslation()
  const { agent } = useAppContext()
  if (!agent) return <InlineStatus>{t("devices:agent.badge.checking")}</InlineStatus>
  return agent.running ? (
    <InlineStatus tone="positive">{t("devices:agent.badge.running")}</InlineStatus>
  ) : (
    <InlineStatus tone="negative">{t("devices:agent.badge.stopped")}</InlineStatus>
  )
}

/** Start button and explanation shown while the agent is not running. */
export function AgentStopped() {
  const { t } = useTranslation()
  const { agent, setAgent } = useAppContext()
  const [busy, setBusy] = useState(false)

  const start = async () => {
    setBusy(true)
    try {
      const connection = await api.startAgent()
      setAgent(connection)
      if (!connection.running) toast.error(t("devices:agent.stoppedPanel.notStartedCheckLog"))
    } catch (e) {
      toastFailure(
        t("devices:agent.stoppedPanel.notStarted"),
        e,
        t("devices:agent.checkLogInDiagnostics"),
      )
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="grid gap-2 py-3">
      <p className="text-sm text-muted-foreground">{t("devices:agent.stoppedPanel.explanation")}</p>
      <div>
        <Button size="sm" disabled={busy || !agent?.canStart} onClick={() => void start()}>
          <Play /> {t("devices:agent.stoppedPanel.start")}
        </Button>
        {agent && !agent.canStart && (
          <p className="mt-2 text-xs text-muted-foreground">
            <Trans
              i18nKey="devices:agent.stoppedPanel.notFound"
              components={{ code: <code className="mx-1" /> }}
            />
          </p>
        )}
      </div>
    </div>
  )
}

/** Features the running agent's protocol does not have, as the user knows them. */
const MISSING_FEATURES: Partial<Record<AgentCapability, TranslationKey>> = {
  encryptedDatabase: "devices:compatibility.feature.encryptedDatabase",
  backgroundExecutions: "devices:compatibility.feature.backgroundExecutions",
  notifications: "devices:compatibility.feature.notifications",
}

/** How a distribution channel is named to the user. */
const CHANNEL_NAMES: Record<string, TranslationKey> = {
  github: "devices:compatibility.channel.github",
  homebrew: "devices:compatibility.channel.homebrew",
  winget: "devices:compatibility.channel.winget",
  apt: "devices:compatibility.channel.apt",
  flathub: "devices:compatibility.channel.flathub",
  mac_app_store: "devices:compatibility.channel.macAppStore",
  microsoft_store: "devices:compatibility.channel.microsoftStore",
}

/** Shown when the running agent is not the one shipped with this app. */
export function AgentCompatibilityNotice() {
  const { t } = useTranslation()
  const format = useFormat()
  const { agent, setAgent } = useAppContext()
  const [busy, setBusy] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const compatibility = agent?.compatibility
  if (!agent?.running || !compatibility || compatibility.type === "compatible") return null
  const running = agent.status?.runningExecutions ?? 0
  const restart = async (interruptRunning: boolean) => {
    setBusy(true)
    try {
      setAgent(await api.restartAgent(interruptRunning))
      toast.success(t("devices:compatibility.restarted"))
    } catch (e) {
      toastFailure(
        t("devices:compatibility.notRestarted"),
        e,
        t("devices:agent.checkLogInDiagnostics"),
      )
    } finally {
      setBusy(false)
    }
  }
  const missing =
    agent.capabilities === undefined
      ? []
      : Object.entries(MISSING_FEATURES)
          .filter(([capability]) => !agent.capabilities?.includes(capability as AgentCapability))
          .map(([, label]) => t(label))
  const channelName = (channel: string) => {
    const key = CHANNEL_NAMES[channel]
    return key ? t(key) : channel
  }
  const required = compatibility.type === "updateRequired"
  const otherInstallation = compatibility.type === "otherDistribution"
  return (
    <Alert variant={required || otherInstallation ? "destructive" : "default"}>
      <TriangleAlert />
      <AlertTitle>
        {required
          ? t("devices:compatibility.title.updateRequired")
          : otherInstallation
            ? t("devices:compatibility.title.otherInstallation")
            : t("devices:compatibility.title.mismatch")}
      </AlertTitle>
      <AlertDescription>
        <p>
          {otherInstallation
            ? t("devices:compatibility.otherInstallation", {
                agent: channelName(compatibility.agent),
                app: channelName(compatibility.app),
              })
            : `${t("devices:compatibility.versions", {
                agent: compatibility.agent,
                app: compatibility.app,
              })} ${
                required
                  ? t("devices:compatibility.replaceRequired")
                  : t("devices:compatibility.restartToMatch")
              }`}
        </p>
        {missing.length > 0 && (
          <p>
            {t("devices:compatibility.unavailable", { features: format.list(missing, "unit") })}
          </p>
        )}
        {running > 0 && <p>{t("devices:compatibility.running", { count: running })}</p>}
        <div className="mt-2 flex gap-2">
          {running > 0 && !confirming ? (
            <Button
              size="sm"
              disabled={busy}
              onClick={() => {
                setConfirming(true)
              }}
            >
              <RotateCw /> {t("devices:compatibility.restartEllipsis")}
            </Button>
          ) : (
            <Button
              size="sm"
              variant={running > 0 ? "destructive" : "default"}
              disabled={busy}
              onClick={() => void restart(running > 0)}
            >
              <RotateCw />{" "}
              {running > 0
                ? t("devices:compatibility.interruptAndRestart")
                : t("devices:compatibility.restart")}
            </Button>
          )}
          {confirming && (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => {
                setConfirming(false)
              }}
            >
              {t("devices:compatibility.wait")}
            </Button>
          )}
        </div>
      </AlertDescription>
    </Alert>
  )
}

export function RefreshDevicesButton() {
  const { t } = useTranslation()
  return (
    <Button
      variant="outline"
      size="sm"
      onClick={() => {
        api.refreshDevices().then(
          () => toast.success(t("devices:refresh.done")),
          (e: unknown) => {
            toastFailure(t("devices:refresh.failed"), e, t("devices:refresh.next"))
          },
        )
      }}
    >
      <RefreshCw /> {t("devices:refresh.label")}
    </Button>
  )
}

export function DeviceRow({ device, action }: { device: DeviceInfo; action?: React.ReactNode }) {
  const { t } = useTranslation()
  const connected = device.state === "connected"
  const layout = device.layout
  // A row of a list: separated from its neighbours by the list, not boxed.
  return (
    <div data-slot="device-row" className="grid gap-0.5 py-2.5">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="text-sm font-medium">{device.modelName}</span>
        <span className="font-mono text-xs text-muted-foreground select-text">
          {device.id.serial}
        </span>
        <InlineStatus tone={connected ? "positive" : "neutral"}>
          {connected ? t("devices:detail.connected") : t("devices:detail.disconnected")}
        </InlineStatus>
        <span className="text-caption text-muted-foreground">
          {device.source === "mock" ? t("devices:detail.simulated") : t("devices:row.usb")}
        </span>
        <div className="flex-1" />
        {action}
      </div>
      <p className="text-caption text-muted-foreground">
        {describeLayout(layout)}
        {connected && device.page && ` · ${t("devices:row.showingPage", { page: device.page })}`}
      </p>
      {layout.type === "invalid" && (
        <ul className="list-disc pl-5 text-xs text-destructive">
          {layout.issues.map((issue) => (
            <li key={issue}>{issue}</li>
          ))}
        </ul>
      )}
    </div>
  )
}

/** Compact connection state for the titlebar; opens the agent settings. */
export function AgentIndicator({ onOpen }: { onOpen: () => void }) {
  const { t } = useTranslation()
  const { agent } = useAppContext()
  const running = agent?.running ?? false
  const label = !agent
    ? t("devices:indicator.checking")
    : running
      ? t("devices:agent.state.running")
      : t("devices:agent.state.stopped")
  return (
    <button
      type="button"
      aria-label={t("devices:indicator.openSettings", { label })}
      title={label}
      className="flex size-(--titlebar-control) shrink-0 items-center justify-center rounded-md hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      onClick={onOpen}
    >
      <span
        className={
          !agent
            ? "size-2 rounded-full bg-muted-foreground/40"
            : running
              ? "size-2 rounded-full bg-emerald-500"
              : "size-2 rounded-full bg-destructive"
        }
        aria-hidden
      />
    </button>
  )
}
