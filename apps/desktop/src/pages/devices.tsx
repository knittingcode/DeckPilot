import { Info } from "lucide-react"

import { ErrorNotice, Loading, Page } from "@/components/page"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { hex } from "@/lib/format"
import type { DeviceModel } from "@/lib/types"

export function DevicesPage() {
  const { state } = useAsync(() => api.listDeviceModels(), "models")

  return (
    <Page title="Devices" description="Supported device models and their capabilities">
      <Alert>
        <Info />
        <AlertTitle>No device communication yet</AlertTitle>
        <AlertDescription>
          Device discovery and input handling will be provided by the background agent. The models
          below are descriptors used for layout validation; none has been tested on physical
          hardware.
        </AlertDescription>
      </Alert>
      {state.status === "loading" && <Loading />}
      {state.status === "error" && <ErrorNotice error={state.error} />}
      {state.status === "ready" &&
        state.data.map((model) => <DeviceModelCard key={model.id} model={model} />)}
    </Page>
  )
}

function DeviceModelCard({ model }: { model: DeviceModel }) {
  const { keys, encoders, touch } = model.capabilities
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          {model.name}
          <Badge variant={model.support === "verified" ? "default" : "outline"}>
            {model.support === "verified" ? "Verified on hardware" : "Descriptor only"}
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
