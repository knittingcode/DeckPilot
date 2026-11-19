import { DndContext } from "@dnd-kit/core"
import { useState } from "react"

import { DeviceCanvas } from "@/components/device-canvas"
import { ErrorNotice, Loading } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { SettingsSection } from "@/components/settings-layout"
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
      <SettingsSection
        title="Supported Models"
        description="Models tested on a physical device on at least one platform. The badges show the status on each platform."
      >
        {state.status === "loading" && <Loading />}
        {state.status === "error" && <ErrorNotice error={state.error} />}
        {state.status === "ready" && supported.length === 0 && (
          <p className="text-sm text-muted-foreground">No model has been verified yet.</p>
        )}
        {supported.length > 0 && (
          <div className="grid divide-y border-y">
            {supported.map((model) => (
              <ModelCard key={model.id} model={model} />
            ))}
          </div>
        )}
        {experimental.length > 0 && (
          <div className="flex items-center gap-2">
            <Switch
              id="show-experimental"
              size="sm"
              checked={showExperimental}
              onCheckedChange={setShowExperimental}
            />
            <Label htmlFor="show-experimental">Show experimental models</Label>
          </div>
        )}
      </SettingsSection>
      {showExperimental && experimental.length > 0 && (
        <SettingsSection
          title="Experimental, not verified"
          description="These models have not been tested on a physical device on any platform. Layouts can be designed for them, but they are not supported."
        >
          <div className="grid divide-y border-y">
            {experimental.map((model) => (
              <ModelCard key={model.id} model={model} />
            ))}
          </div>
        </SettingsSection>
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
            variant={entry.level === "physicallyVerified" ? "secondary" : "outline"}
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
    // Key gestures come from DeckPilot's gesture engine and work on every
    // model with keys; touch gestures are reported by the touch strip.
    ["Key gestures", keys ? "Click, double click, long press" : "—"],
    ["Touch gestures", touch ? touch.gestures.map((g) => GESTURES[g]).join(", ") : "—"],
    ["USB identity", model.usb ? `${hex(model.usb.vendorId)}:${hex(model.usb.productId)}` : "—"],
  ]
  return (
    <article aria-label={model.name} className="grid gap-3 py-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="grid gap-0.5">
          <h4 className="text-sm leading-5 font-medium">{model.name}</h4>
          <p className="text-caption text-muted-foreground">
            {model.vendor} · <span className="font-mono">{model.id}</span>
          </p>
        </div>
        <PlatformBadges model={model} />
      </div>
      <div className="grid gap-6 lg:grid-cols-[auto_1fr]">
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
                  <dd className="text-caption text-muted-foreground">
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
      </div>
    </article>
  )
}
