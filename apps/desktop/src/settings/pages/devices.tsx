import { LayoutGrid, RefreshCw } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { AgentBadge, AgentCompatibilityNotice, AgentStopped, DeviceRow } from "@/components/agent"
import { GestureDefaultsCard } from "@/components/gesture-editor/gesture-defaults"
import { SupportedModels } from "@/components/supported-models"
import { EmptyState, ErrorNotice } from "@/components/page"
import { SettingsGroup, SettingsRow, SettingsSection } from "@/components/settings-layout"
import { Button } from "@/components/ui/button"
import { useTitlebar } from "@/hooks/use-titlebar"
import { api } from "@/lib/api"
import { failureMessage, toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"

export function DevicesSettings() {
  const { t } = useTranslation()
  const { agent, navigate } = useAppContext()
  const [refreshing, setRefreshing] = useState(false)
  useTitlebar({
    actions: [
      {
        id: "devices.refresh",
        label: t("devices:refresh.label"),
        icon: RefreshCw,
        placement: "primary",
        busy: refreshing,
        disabled: !agent?.running || refreshing,
        hint: agent?.running ? t("devices:page.refreshHint") : t("devices:detail.agentStopped"),
        onSelect: () => {
          setRefreshing(true)
          api
            .refreshDevices()
            .then(
              () => toast.success(t("devices:refresh.done")),
              (e: unknown) => {
                toastFailure(t("devices:refresh.failed"), e, t("devices:refresh.next"))
              },
            )
            .finally(() => {
              setRefreshing(false)
            })
        },
      },
    ],
  })
  return (
    <>
      <AgentCompatibilityNotice />
      <SettingsSection
        title={
          <>
            {t("devices:page.connectedTitle")} <AgentBadge />
          </>
        }
        description={t("devices:page.connectedDescription")}
      >
        <SettingsGroup>
          {agent && !agent.running && <AgentStopped />}
          {agent?.status &&
            (agent.status.devices.length === 0 ? (
              <EmptyState variant="plain" title={t("devices:page.noneDetected")} />
            ) : (
              agent.status.devices.map((device) => (
                <DeviceRow
                  key={device.id.serial}
                  device={device}
                  action={
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => {
                        navigate("designer")
                      }}
                    >
                      <LayoutGrid /> {t("devices:page.editLayout")}
                    </Button>
                  }
                />
              ))
            ))}
        </SettingsGroup>
      </SettingsSection>
      {agent?.status && (
        <SettingsSection title={t("devices:page.display")}>
          <SettingsGroup>
            <Brightness key={agent.status.brightness} current={agent.status.brightness} />
          </SettingsGroup>
        </SettingsSection>
      )}
      <GestureDefaultsCard />
      <SupportedModels />
    </>
  )
}

function Brightness({ current }: { current: number }) {
  const { t } = useTranslation()
  const format = useFormat()
  const [brightness, setBrightness] = useState(current)
  const [error, setError] = useState<string | null>(null)
  const percent = format.percent(brightness / 100)
  return (
    <>
      <SettingsRow
        label={t("devices:page.brightness")}
        htmlFor="brightness"
        description={t("devices:page.brightnessDescription")}
      >
        <input
          id="brightness"
          type="range"
          min={0}
          max={100}
          step={5}
          value={brightness}
          aria-valuetext={percent}
          className="w-40 accent-primary"
          onChange={(e) => {
            setBrightness(Number(e.target.value))
          }}
        />
        <output htmlFor="brightness" className="w-10 text-right text-sm tabular-nums">
          {percent}
        </output>
        <Button
          size="sm"
          variant="outline"
          disabled={brightness === current}
          onClick={() => {
            setError(null)
            api.setBrightness(brightness).catch((e: unknown) => {
              setError(failureMessage(t("devices:page.brightnessFailed"), e))
            })
          }}
        >
          {t("devices:page.apply")}
        </Button>
      </SettingsRow>
      {error && (
        <div className="py-2">
          <ErrorNotice error={error} />
        </div>
      )}
    </>
  )
}
