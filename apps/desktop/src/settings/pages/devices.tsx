import { LayoutGrid, RefreshCw } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { AgentBadge, AgentCompatibilityNotice, AgentStopped, DeviceRow } from "@/components/agent"
import { GestureDefaultsCard } from "@/components/gesture-editor/gesture-defaults"
import { SupportedModels } from "@/components/supported-models"
import { EmptyState, ErrorNotice } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Label } from "@/components/ui/label"
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
      <Card>
        <CardHeader className="flex flex-row items-start gap-2">
          <div className="grid flex-1 gap-1.5">
            <CardTitle className="flex items-center gap-2">
              Connected devices <AgentBadge />
            </CardTitle>
            <CardDescription>
              Devices appear automatically when connected. Simulated devices are for development and
              testing. The Stream Deck MK.2 driver has not been verified on hardware yet; on Linux,
              a udev rule is required for access.
            </CardDescription>
          </div>
        </CardHeader>
        <CardContent className="grid gap-2">
          {agent && !agent.running && <AgentStopped />}
          {agent?.status &&
            (agent.status.devices.length === 0 ? (
              <EmptyState title="No Stream Deck detected" />
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
        </CardContent>
      </Card>
      {agent?.status && (
        <Brightness key={agent.status.brightness} current={agent.status.brightness} />
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
    <Card>
      <CardHeader>
        <CardTitle>Brightness</CardTitle>
        <CardDescription>Applies to every connected device.</CardDescription>
      </CardHeader>
      <CardContent className="grid max-w-md gap-2">
        {error && <ErrorNotice error={error} />}
        <Label htmlFor="brightness">Device brightness: {brightness}%</Label>
        <div className="flex items-center gap-3">
          <input
            id="brightness"
            type="range"
            min={0}
            max={100}
            step={5}
            value={brightness}
            className="flex-1 accent-primary"
            onChange={(e) => {
              setBrightness(Number(e.target.value))
            }}
          />
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
        </div>
      </CardContent>
    </Card>
  )
}
