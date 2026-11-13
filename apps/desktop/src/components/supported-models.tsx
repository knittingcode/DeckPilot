import { DndContext } from "@dnd-kit/core"

import { DeviceCanvas } from "@/components/device-canvas"
import { ErrorNotice, Loading } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { SUPPORT, hex } from "@/lib/format"
import type { DeviceModel, TouchGesture } from "@/lib/types"

/**
 * The device models of the catalog, in Settings › Devices (formerly the
 * "Device Models" page of the main navigation).
 */
export function SupportedModels() {
  const { state } = useAsync(() => api.listDeviceModels(), "models")
  return (
    <section aria-labelledby="supported-models" className="grid gap-4">
      <div className="grid gap-1">
        <h2 id="supported-models" className="text-base font-semibold">
          Supported Models
        </h2>
        <p className="text-sm text-muted-foreground">
          Models you can design layouts for, with or without hardware.
        </p>
      </div>
      {state.status === "loading" && <Loading />}
      {state.status === "error" && <ErrorNotice error={state.error} />}
      {state.status === "ready" && (
        <DndContext>
          {state.data.map((model) => (
            <ModelCard key={model.id} model={model} />
          ))}
        </DndContext>
      )}
    </section>
  )
}

const GESTURES: Record<TouchGesture, string> = {
  tap: "Tap",
  longPress: "Long press",
  swipe: "Swipe",
}

function ModelCard({ model }: { model: DeviceModel }) {
  const { navigate } = useAppContext()
  const { keys, encoders, touch } = model.capabilities
  const rows: [string, string][] = [
    [
      "Keys",
      keys
        ? `${keys.rows * keys.columns} (${keys.columns} × ${keys.rows})${
            keys.image
              ? `, ${keys.image.width} × ${keys.image.height} px ${keys.image.format.toUpperCase()}`
              : ""
          }`
        : "None",
    ],
    ["Dials", encoders ? `${encoders.count}${encoders.pressable ? ", pressable" : ""}` : "None"],
    [
      "Touch strip",
      touch
        ? `${touch.zones} zones${touch.image ? ` of ${touch.image.width} × ${touch.image.height} px` : ""}`
        : "None",
    ],
    ["Gestures", touch ? touch.gestures.map((g) => GESTURES[g]).join(", ") : "—"],
    ["USB identity", model.usb ? `${hex(model.usb.vendorId)}:${hex(model.usb.productId)}` : "—"],
  ]
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          {model.name}
          <Badge variant={model.support === "verified" ? "default" : "outline"}>
            {SUPPORT[model.support].label}
          </Badge>
        </CardTitle>
        <CardDescription>
          {model.vendor} · <span className="font-mono">{model.id}</span> ·{" "}
          {SUPPORT[model.support].detail}
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-6 lg:grid-cols-[auto_1fr]">
        <div className="overflow-x-auto">
          <DeviceCanvas model={model} size="sm" />
        </div>
        <div className="grid h-fit gap-4">
          <dl className="grid grid-cols-[8rem_1fr] gap-x-4 gap-y-1 text-sm">
            {rows.map(([label, value]) => (
              <div key={label} className="contents">
                <dt className="text-muted-foreground">{label}</dt>
                <dd className={label === "USB identity" ? "font-mono" : undefined}>{value}</dd>
              </div>
            ))}
          </dl>
          <div>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                navigate("designer")
              }}
            >
              Design a layout
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>
  )
}
