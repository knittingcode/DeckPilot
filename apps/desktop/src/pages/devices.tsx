import { LayoutGrid } from "lucide-react"

import {
  AgentBadge,
  AgentCompatibilityNotice,
  AgentStopped,
  DeviceRow,
  RefreshDevicesButton,
} from "@/components/agent"
import { EmptyState, Page } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useAppContext } from "@/lib/app-context"

export function DevicesPage() {
  const { agent, navigate } = useAppContext()
  return (
    <Page title="Connected Devices" description="Devices the background agent is driving">
      <AgentCompatibilityNotice />
      <Card>
        <CardHeader className="flex flex-row items-start gap-2">
          <div className="grid flex-1 gap-1.5">
            <CardTitle className="flex items-center gap-2">
              Devices <AgentBadge />
            </CardTitle>
            <CardDescription>
              Devices appear automatically when connected. Simulated devices are for development and
              testing. The Stream Deck MK.2 driver has not been verified on hardware yet; on Linux,
              a udev rule is required for access.
            </CardDescription>
          </div>
          {agent?.running && <RefreshDevicesButton />}
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
    </Page>
  )
}
