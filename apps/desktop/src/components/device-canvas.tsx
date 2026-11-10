import { useDroppable } from "@dnd-kit/core"

import { FaceTile } from "@/components/key-face"
import { controlKey, controlLabel, controlsOf, sameControl } from "@/lib/layout-model"
import type { ControlId, DeviceModel, KeyFace } from "@/lib/types"
import { cn } from "@/lib/utils"

interface CanvasProps {
  model: DeviceModel
  faces?: Map<string, KeyFace>
  icons?: Map<string, string | null | undefined>
  /** Text for controls without a display (dials). */
  captions?: Map<string, string>
  selected?: ControlId | null
  onSelect?: (control: ControlId) => void
  /** Controls accept drops from the action palette. */
  droppable?: boolean
  size?: "sm" | "md"
}

function Control({
  control,
  shape,
  props,
}: {
  control: ControlId
  shape: "key" | "zone" | "dial"
  props: CanvasProps
}) {
  const key = controlKey(control)
  const { setNodeRef, isOver } = useDroppable({ id: key, disabled: !props.droppable })
  const selected = props.selected ? sameControl(props.selected, control) : false
  const interactive = Boolean(props.onSelect)
  const face = props.faces?.get(key)
  return (
    <button
      ref={setNodeRef}
      type="button"
      disabled={!interactive}
      aria-label={controlLabel(control)}
      aria-pressed={interactive ? selected : undefined}
      title={
        shape === "dial" && props.captions?.get(key)
          ? `${controlLabel(control)}: ${props.captions.get(key) ?? ""}`
          : controlLabel(control)
      }
      className={cn(
        "relative block w-full min-w-0 outline-none disabled:cursor-default",
        shape === "key" && "aspect-square rounded-lg",
        shape === "zone" && "aspect-[2/1] rounded-md",
        shape === "dial" && "aspect-square rounded-full",
        "ring-offset-2 ring-offset-neutral-900 focus-visible:ring-2 focus-visible:ring-sky-400",
        selected && "ring-2 ring-amber-400",
        isOver && "ring-2 ring-emerald-400",
      )}
      onClick={() => props.onSelect?.(control)}
    >
      {shape === "dial" ? (
        <div className="flex size-full flex-col items-center justify-center rounded-full border-4 border-neutral-600 bg-neutral-800 p-1 text-center text-[10px] leading-tight text-neutral-200">
          <span className="line-clamp-2">
            {(props.captions?.get(key) ?? "").split(" · ")[0]?.replace(/^[^:]*: /, "")}
          </span>
        </div>
      ) : (
        <FaceTile
          face={face}
          icon={props.icons?.get(key)}
          compact={shape === "zone"}
          className={shape === "zone" ? "rounded-md" : "rounded-lg"}
        />
      )}
    </button>
  )
}

/** A device drawn from its capabilities: keys, touch zones and dials. */
export function DeviceCanvas(props: CanvasProps) {
  const { model, size = "md" } = props
  const { keys, columns, encoders, touchZones } = controlsOf(model)
  const unit = size === "sm" ? 2.25 : 4.25
  // The touch strip and the dials span the key grid, as on the hardware.
  const width = columns > 0 ? `${columns * unit + (columns - 1) * 0.5}rem` : `${unit * 4}rem`
  return (
    <div
      className="inline-grid gap-3 rounded-2xl border border-neutral-700 bg-neutral-900 p-4 shadow-inner dark:border-neutral-600 dark:bg-black"
      role="group"
      aria-label={`${model.name} controls`}
    >
      {keys.length > 0 && (
        <div
          className="grid gap-2"
          style={{ gridTemplateColumns: `repeat(${columns}, ${unit}rem)` }}
        >
          {keys.map(({ control }) => (
            <Control key={controlKey(control)} control={control} shape="key" props={props} />
          ))}
        </div>
      )}
      {touchZones.length > 0 && (
        <div
          className="grid gap-1 rounded-lg bg-neutral-950 p-1"
          style={{ width, gridTemplateColumns: `repeat(${touchZones.length}, minmax(0, 1fr))` }}
        >
          {touchZones.map((control) => (
            <Control key={controlKey(control)} control={control} shape="zone" props={props} />
          ))}
        </div>
      )}
      {encoders.length > 0 && (
        <div
          className="grid justify-items-center gap-2"
          style={{ width, gridTemplateColumns: `repeat(${encoders.length}, minmax(0, 1fr))` }}
        >
          {encoders.map((control) => (
            <div key={controlKey(control)} style={{ width: `${unit * 0.9}rem` }}>
              <Control control={control} shape="dial" props={props} />
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
