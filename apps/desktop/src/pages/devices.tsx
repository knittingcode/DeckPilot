import { Info } from "lucide-react"
import { useState } from "react"

import { AgentBadge, AgentStopped, DeviceRow, RefreshDevicesButton } from "@/components/agent"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useAsync } from "@/hooks/use-async"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { hex } from "@/lib/format"
import type { DeviceInfo, DeviceModel } from "@/lib/types"

export function DevicesPage() {
  const { state } = useAsync(() => api.listDeviceModels(), "models")

  return (
    <Page title="Devices" description="Connected devices and supported models">
      <ConnectedDevices />
      <Alert>
        <Info />
        <AlertTitle>Hardware support is not yet verified</AlertTitle>
        <AlertDescription>
          The Stream Deck MK.2 driver has not been tested on a physical device yet. On Linux, access
          to the device requires a udev rule; see the documentation.
        </AlertDescription>
      </Alert>
      {state.status === "loading" && <Loading />}
      {state.status === "error" && <ErrorNotice error={state.error} />}
      {state.status === "ready" &&
        state.data.map((model) => <DeviceModelCard key={model.id} model={model} />)}
    </Page>
  )
}

function ConnectedDevices() {
  const { agent, profiles, refresh } = useAppContext()
  const [error, setError] = useState<string | null>(null)
  const activeProfile = profiles.activeProfileId

  const generate = (device: DeviceInfo) => {
    if (!activeProfile) return
    setError(null)
    api.createDefaultLayout(activeProfile, device.id.model).then(refresh, (e: unknown) => {
      setError(errorMessage(e))
    })
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          Connected devices <AgentBadge />
        </CardTitle>
        <CardDescription>
          Devices are discovered by the background agent. A generated layout places the active
          profile&apos;s actions on the keys in order; a layout editor is planned.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-2">
        {error && <ErrorNotice error={error} />}
        {agent && !agent.running && <AgentStopped />}
        {agent?.status && (
          <>
            {agent.status.devices.length === 0 ? (
              <EmptyState title="No Stream Deck detected" />
            ) : (
              agent.status.devices.map((device) => (
                <DeviceRow key={device.id.serial} device={device} onGenerateLayout={generate} />
              ))
            )}
            <div>
              <RefreshDevicesButton />
            </div>
          </>
        )}
      </CardContent>
    </Card>
  )
}

const SUPPORT_LABELS: Record<DeviceModel["support"], string> = {
  verified: "Verified on hardware",
  unverified: "Driver not yet tested on hardware",
  descriptorOnly: "Descriptor only",
}

function DeviceModelCard({ model }: { model: DeviceModel }) {
  const { keys, encoders, touch } = model.capabilities
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          {model.name}
          <Badge variant={model.support === "verified" ? "default" : "outline"}>
            {SUPPORT_LABELS[model.support]}
          </Badge>
        </CardTitle>
        <CardDescription>
          {model.vendor} · <span className="font-mono">{model.id}</span>
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-6 md:grid-cols-[auto_1fr]">
        {keys && (
          <div
            className="grid w-fit gap-2 rounded-xl bg-neutral-900 p-3"
            style={{ gridTemplateColumns: `repeat(${keys.columns}, minmax(0, 1fr))` }}
            aria-label={`${keys.columns} by ${keys.rows} key grid`}
          >
            {Array.from({ length: keys.rows * keys.columns }, (_, index) => (
              <div
                key={index}
                className="flex size-10 items-center justify-center rounded-md bg-neutral-700 text-[10px] text-neutral-300 tabular-nums"
              >
                {index}
              </div>
            ))}
          </div>
        )}
        <dl className="grid h-fit grid-cols-[10rem_1fr] gap-x-4 gap-y-1 text-sm">
          <dt className="text-muted-foreground">Keys</dt>
          <dd>
            {keys
              ? `${keys.rows * keys.columns} (${keys.columns} × ${keys.rows})${
                  keys.image
                    ? `, ${keys.image.width} × ${keys.image.height} px ${keys.image.format.toUpperCase()}`
                    : ""
                }`
              : "None"}
          </dd>
          <dt className="text-muted-foreground">Encoders</dt>
          <dd>
            {encoders ? `${encoders.count}${encoders.pressable ? ", pressable" : ""}` : "None"}
          </dd>
          <dt className="text-muted-foreground">Touch surface</dt>
          <dd>{touch ? `${touch.zones} zones (${touch.gestures.join(", ")})` : "None"}</dd>
          <dt className="text-muted-foreground">USB identity</dt>
          <dd className="font-mono">
            {model.usb ? `${hex(model.usb.vendorId)}:${hex(model.usb.productId)}` : "—"}
          </dd>
        </dl>
      </CardContent>
    </Card>
  )
}
