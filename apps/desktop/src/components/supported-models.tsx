import { DndContext } from "@dnd-kit/core"
import { useState } from "react"

import { DeviceCanvas } from "@/components/device-canvas"
import { ErrorNotice, Loading } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { hex } from "@/lib/format"
import type { DeviceModel, TouchGesture } from "@/lib/types"
import { LEVELS, PLATFORMS, PLATFORM_LABELS, isSupported, verificationOn } from "@/lib/verification"

/**
 * The device models of the catalog, in Settings › Devices (formerly the
 * "Device Models" page of the main navigation). Only models tested on a
 * physical device on at least one platform are listed as supported; the
 * others are shown on request, labelled as experimental and not verified.
 */
export function SupportedModels() {
  const { state } = useAsync(() => api.listDeviceModels(), "models")
  const [showExperimental, setShowExperimental] = useState(false)
  const models = state.status === "ready" ? state.data : []
  const supported = models.filter(isSupported)
  const experimental = models.filter((model) => !isSupported(model))
  return (
    <DndContext>
      <section aria-labelledby="supported-models" className="grid gap-4">
        <div className="grid gap-1">
          <h2 id="supported-models" className="text-base font-semibold">
            Supported Models
          </h2>
          <p className="text-sm text-muted-foreground">
            Models tested on a physical device on at least one platform. The badges show the status
            on each platform.
          </p>
        </div>
        {state.status === "loading" && <Loading />}
        {state.status === "error" && <ErrorNotice error={state.error} />}
        {state.status === "ready" && supported.length === 0 && (
          <p className="text-sm text-muted-foreground">No model has been verified yet.</p>
        )}
        {supported.map((model) => (
          <ModelCard key={model.id} model={model} />
        ))}
        {experimental.length > 0 && (
          <div className="flex items-center gap-2">
            <Switch
              id="show-experimental"
              checked={showExperimental}
              onCheckedChange={setShowExperimental}
            />
            <Label htmlFor="show-experimental">Show experimental models</Label>
          </div>
        )}
      </section>
      {showExperimental && experimental.length > 0 && (
        <section aria-labelledby="experimental-models" className="grid gap-4">
          <div className="grid gap-1">
            <h2 id="experimental-models" className="text-base font-semibold">
              Experimental, not verified
            </h2>
            <p className="text-sm text-muted-foreground">
              These models have not been tested on a physical device on any platform. Layouts can be
              designed for them, but they are not supported.
            </p>
          </div>
          {experimental.map((model) => (
            <ModelCard key={model.id} model={model} />
          ))}
        </section>
      )}
    </DndContext>
  )
}

/** One badge per platform with the model's verification level there. */
export function PlatformBadges({ model }: { model: DeviceModel }) {
  return (
    <span className="flex flex-wrap gap-1">
      {PLATFORMS.map((platform) => {
        const entry = verificationOn(model, platform)
        return (
          <Badge
            key={platform}
            variant={entry.level === "physicallyVerified" ? "default" : "outline"}
            title={entry.evidence || LEVELS[entry.level].detail}
          >
            {PLATFORM_LABELS[platform]}: {LEVELS[entry.level].label}
          </Badge>
        )
      })}
    </span>
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
          <PlatformBadges model={model} />
        </CardTitle>
        <CardDescription>
          {model.vendor} · <span className="font-mono">{model.id}</span>
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
          <dl className="grid gap-1 text-sm" aria-label="Verification">
            {PLATFORMS.map((platform) => {
              const entry = verificationOn(model, platform)
              return (
                <div key={platform}>
                  <dt className="font-medium">
                    {PLATFORM_LABELS[platform]}
                    {entry.evidenceDate && (
                      <span className="font-normal text-muted-foreground">
                        {" "}
                        (as of {entry.evidenceDate})
                      </span>
                    )}
                  </dt>
                  <dd className="text-muted-foreground">
                    {entry.evidence || LEVELS[entry.level].detail}
                  </dd>
                </div>
              )
            })}
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
