import { useDraggable, useDroppable } from "@dnd-kit/core"
import { useRef, type KeyboardEvent, type ReactElement, type ReactNode } from "react"

import { FaceTile } from "@/components/key-face"
import { IS_MAC } from "@/hooks/use-navigation"
import {
  neighbour,
  parseControlKey,
  type Direction,
  type SelectMode,
} from "@/layout-editor/editing"
import { controlKey, controlLabel, controlsOf, sameControl } from "@/lib/layout-model"
import type { ControlId, DeviceModel, KeyFace } from "@/lib/types"
import { cn } from "@/lib/utils"

interface CanvasProps {
  model: DeviceModel
  faces?: Map<string, KeyFace>
  /** Rendered key images (PNG data URLs); shown instead of the drawn face. */
  images?: Map<string, string>
  icons?: Map<string, string | null | undefined>
  /** Text for controls without a display (dials). */
  captions?: Map<string, string>
  /** What each bound control does; shown as its tooltip and description. */
  summaries?: Map<string, string>
  /** Selected controls. */
  selection?: ControlId[]
  /** The control that takes keyboard focus (roving tab stop). */
  focused?: ControlId | null
  onSelect?: (control: ControlId, mode: SelectMode) => void
  /** Keys other than arrows pressed on a control (Delete, Escape, …). */
  onControlKeyDown?: (event: KeyboardEvent<HTMLButtonElement>, control: ControlId) => void
  /** Controls accept drops from the action palette. */
  droppable?: boolean
  /** Controls can be dragged onto other controls. */
  draggable?: boolean
  /** Wraps each control, e.g. in a context menu. */
  wrapControl?: (control: ControlId, element: ReactElement) => ReactNode
  size?: "sm" | "md"
}

const ARROWS: Record<string, Direction> = {
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
}

function selectMode(event: { shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }): SelectMode {
  if (event.shiftKey) return "extend"
  if (IS_MAC ? event.metaKey : event.ctrlKey) return "toggle"
  return "only"
}

function Control({
  control,
  shape,
  tabStop,
  props,
}: {
  control: ControlId
  shape: "key" | "zone" | "dial"
  tabStop: boolean
  props: CanvasProps
}) {
  const key = controlKey(control)
  const drop = useDroppable({ id: key, disabled: !props.droppable })
  const drag = useDraggable({
    id: `control:${key}`,
    data: { control },
    disabled: !props.draggable,
  })
  const selected = props.selection?.some((c) => sameControl(c, control)) ?? false
  const interactive = Boolean(props.onSelect)
  const face = props.faces?.get(key)
  const image = props.images?.get(key)
  const summary = props.summaries?.get(key) ?? (shape === "dial" ? props.captions?.get(key) : "")
  // Only pointer dragging: Enter and Space keep selecting the control.
  const onPointerDown = props.draggable
    ? (drag.listeners?.onPointerDown as React.PointerEventHandler | undefined)
    : undefined
  const button = (
    <button
      ref={(node) => {
        drop.setNodeRef(node)
        drag.setNodeRef(node)
      }}
      type="button"
      data-control={key}
      disabled={!interactive}
      tabIndex={interactive ? (tabStop ? 0 : -1) : undefined}
      aria-label={controlLabel(control)}
      aria-pressed={interactive ? selected : undefined}
      title={summary ? `${controlLabel(control)}: ${summary}` : controlLabel(control)}
      className={cn(
        "relative block w-full min-w-0 outline-none disabled:cursor-default",
        shape === "key" && "aspect-square rounded-lg",
        shape === "zone" && "aspect-[2/1] rounded-md",
        shape === "dial" && "aspect-square rounded-full",
        "ring-offset-2 ring-offset-neutral-900 focus-visible:ring-2 focus-visible:ring-sky-400",
        selected && "ring-2 ring-amber-400",
        drop.isOver && "ring-2 ring-emerald-400",
        drag.isDragging && "opacity-50",
      )}
      onPointerDown={onPointerDown}
      onClick={(event) => props.onSelect?.(control, selectMode(event))}
      onKeyDown={(event) => {
        props.onControlKeyDown?.(event, control)
      }}
    >
      {shape === "dial" ? (
        <div className="flex size-full flex-col items-center justify-center rounded-full border-4 border-neutral-600 bg-neutral-800 p-1 text-center text-[10px] leading-tight text-neutral-200">
          <span className="line-clamp-2">
            {(props.captions?.get(key) ?? "").split(" · ")[0]?.replace(/^[^:]*: /, "")}
          </span>
        </div>
      ) : image ? (
        // The key as the device draws it; the button carries the name.
        <img
          src={image}
          alt=""
          className={cn(
            "size-full bg-black object-cover",
            shape === "zone" ? "rounded-md" : "rounded-lg",
          )}
        />
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
  return props.wrapControl ? props.wrapControl(control, button) : button
}

/** A device drawn from its capabilities: keys, touch zones and dials. */
export function DeviceCanvas(props: CanvasProps) {
  const { model, size = "md", onSelect, focused } = props
  const { keys, columns, encoders, touchZones } = controlsOf(model)
  const container = useRef<HTMLDivElement>(null)
  const unit = size === "sm" ? 2.25 : 4.25
  // The touch strip and the dials span the key grid, as on the hardware.
  const width = columns > 0 ? `${columns * unit + (columns - 1) * 0.5}rem` : `${unit * 4}rem`
  const first = keys[0]?.control ?? touchZones[0] ?? encoders[0]
  const stop = focused ?? first
  const isStop = (control: ControlId) => (stop ? sameControl(stop, control) : false)

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const direction = ARROWS[event.key]
    const from = (event.target as HTMLElement).dataset.control
    const current = from ? parseControlKey(from) : null
    if (!direction || !onSelect || !current) return
    const next = neighbour(model, current, direction)
    event.preventDefault()
    onSelect(next, event.shiftKey ? "extend" : "only")
    container.current
      ?.querySelector<HTMLButtonElement>(`[data-control="${controlKey(next)}"]`)
      ?.focus()
  }

  return (
    <div
      ref={container}
      className="inline-grid gap-3 rounded-2xl border border-neutral-700 bg-neutral-900 p-4 shadow-inner dark:border-neutral-600 dark:bg-black"
      role="group"
      aria-label={`${model.name} controls`}
      onKeyDown={onKeyDown}
    >
      {keys.length > 0 && (
        <div
          className="grid gap-2"
          style={{ gridTemplateColumns: `repeat(${columns}, ${unit}rem)` }}
        >
          {keys.map(({ control }) => (
            <Control
              key={controlKey(control)}
              control={control}
              shape="key"
              tabStop={isStop(control)}
              props={props}
            />
          ))}
        </div>
      )}
      {touchZones.length > 0 && (
        <div
          className="grid gap-1 rounded-lg bg-neutral-950 p-1"
          style={{ width, gridTemplateColumns: `repeat(${touchZones.length}, minmax(0, 1fr))` }}
        >
          {touchZones.map((control) => (
            <Control
              key={controlKey(control)}
              control={control}
              shape="zone"
              tabStop={isStop(control)}
              props={props}
            />
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
              <Control control={control} shape="dial" tabStop={isStop(control)} props={props} />
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
