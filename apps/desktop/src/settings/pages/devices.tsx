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

export function DevicesSettings() {
  const { agent, navigate } = useAppContext()
  const [refreshing, setRefreshing] = useState(false)
  useTitlebar({
    actions: [
      {
        id: "devices.refresh",
        label: "Refresh devices",
        icon: RefreshCw,
        placement: "primary",
        busy: refreshing,
        disabled: !agent?.running || refreshing,
        hint: agent?.running ? "Look for connected devices again" : "The agent is not running",
        onSelect: () => {
          setRefreshing(true)
          api
            .refreshDevices()
            .then(
              () => toast.success("Devices refreshed"),
              (e: unknown) => {
                toastFailure("The devices were not refreshed", e, "Check that the agent is running")
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
            Connected devices <AgentBadge />
          </>
        }
        description="Devices appear automatically when connected. Simulated devices are for development and testing. On Linux, a udev rule is required for access."
      >
        <SettingsGroup>
          {agent && !agent.running && <AgentStopped />}
          {agent?.status &&
            (agent.status.devices.length === 0 ? (
              <EmptyState variant="plain" title="No Stream Deck detected" />
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
                      <LayoutGrid /> Edit layout
                    </Button>
                  }
                />
              ))
            ))}
        </SettingsGroup>
      </SettingsSection>
      {agent?.status && (
        <SettingsSection title="Display">
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
  const [brightness, setBrightness] = useState(current)
  const [error, setError] = useState<string | null>(null)
  return (
    <>
      <SettingsRow
        label="Brightness"
        htmlFor="brightness"
        description="Applies to every connected device."
      >
        <input
          id="brightness"
          type="range"
          min={0}
          max={100}
          step={5}
          value={brightness}
          aria-valuetext={`${brightness}%`}
          className="w-40 accent-primary"
          onChange={(e) => {
            setBrightness(Number(e.target.value))
          }}
        />
        <output htmlFor="brightness" className="w-10 text-right text-sm tabular-nums">
          {brightness}%
        </output>
        <Button
          size="sm"
          variant="outline"
          disabled={brightness === current}
          onClick={() => {
            setError(null)
            api.setBrightness(brightness).catch((e: unknown) => {
              setError(failureMessage("The brightness was not changed", e))
            })
          }}
        >
          Apply
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
