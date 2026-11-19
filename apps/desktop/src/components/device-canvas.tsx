import { useDraggable, useDroppable } from "@dnd-kit/core"
import {
  useRef,
  type CSSProperties,
  type KeyboardEvent,
  type ReactElement,
  type ReactNode,
} from "react"

import {
  BODY_PADDING,
  DIAL_SIZE,
  KEY_GAP,
  deviceGeometry,
  gridWidth,
} from "@/components/device-geometry"
import { FaceTile } from "@/components/key-face"
import { IS_MAC } from "@/hooks/use-navigation"
import {
  neighbour,
  parseControlKey,
  type Direction,
  type DragSource,
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
  /**
   * Keys that cannot do anything now, with why in a word or two (e.g. "No
   * previous layout"): drawn dimmed with that detail, as on the device.
   */
  unavailable?: Map<string, string>
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
  /** A control was double-clicked, e.g. to enter the folder it opens. */
  onOpen?: (control: ControlId) => void
  /** Keys other than arrows pressed on a control (Delete, Escape, …). */
  onControlKeyDown?: (event: KeyboardEvent<HTMLButtonElement>, control: ControlId) => void
  /** Controls accept drops from the action palette. */
  droppable?: boolean
  /** Controls can be dragged onto other controls. */
  draggable?: boolean
  /**
   * Why what is dragged (its drag data) cannot be dropped on `control`;
   * `null` when it can. A control that refuses shows it while the drag is
   * over it.
   */
  dropRejection?: (control: ControlId, dragged: unknown) => string | null
  /** A control that just refused a drop, shown as refused for a moment. */
  rejected?: ControlId | null
  /** Wraps each control, e.g. in a context menu. */
  wrapControl?: (control: ControlId, element: ReactElement) => ReactNode
  size?: "sm" | "md"
  /**
   * The size of one key in CSS pixels, e.g. from `keySize` to fit the
   * space the preview has; everything else scales with it. Without it, the
   * `size` step.
   */
  keySize?: number
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

/**
 * What every control shares: a dark LCD behind a hairline border, a subtle
 * hover and the keyboard focus outline, offset beyond the selection ring.
 */
const KEY_BASE = cn(
  "relative block w-full min-w-0 overflow-hidden border border-device-key-border bg-device-key",
  "transition-[border-color,filter,box-shadow] duration-100 disabled:cursor-default",
  "ring-offset-1 ring-offset-device-body",
  "enabled:hover:border-device-key-hover enabled:hover:brightness-110",
  "outline-none focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-device-focus focus-visible:outline-solid",
)

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
  // What the editor's drop handler reads: without the type it took a
  // dragged control for a library entry.
  const source: DragSource = { type: "control", control }
  const drag = useDraggable({
    id: `control:${key}`,
    data: source,
    disabled: !props.draggable,
  })
  const selected = props.selection?.some((c) => sameControl(c, control)) ?? false
  const interactive = Boolean(props.onSelect)
  const face = props.faces?.get(key)
  const unavailable = props.unavailable?.get(key)
  const image = unavailable === undefined ? props.images?.get(key) : undefined
  const described = props.summaries?.get(key) ?? (shape === "dial" ? props.captions?.get(key) : "")
  const summary = unavailable && described ? `${described} (${unavailable})` : described
  const refusal =
    drop.isOver && props.dropRejection
      ? props.dropRejection(control, drop.active?.data.current)
      : null
  const refused =
    refusal !== null || (props.rejected ? sameControl(props.rejected, control) : false)
  // Only pointer dragging: Enter and Space keep selecting the control.
  const onPointerDown = props.draggable
    ? (drag.listeners?.onPointerDown as React.PointerEventHandler | undefined)
    : undefined
  // Selection, an accepted and a refused drop are rings around the key
  // (box shadows) and keyboard focus is an outline outside them: none of
  // them changes the key's size or the layout.
  const mark = refused
    ? "ring-2 ring-destructive"
    : drop.isOver
      ? "ring-2 ring-device-drop"
      : selected
        ? "ring-2 ring-device-selected"
        : undefined
  const button = (
    <button
      ref={(node) => {
        drop.setNodeRef(node)
        drag.setNodeRef(node)
      }}
      type="button"
      data-control={key}
      data-shape={shape}
      data-drop={refused ? "rejected" : drop.isOver ? "accepted" : undefined}
      disabled={!interactive}
      tabIndex={interactive ? (tabStop ? 0 : -1) : undefined}
      aria-label={controlLabel(control)}
      aria-pressed={interactive ? selected : undefined}
      title={summary ? `${controlLabel(control)}: ${summary}` : controlLabel(control)}
      className={cn(
        KEY_BASE,
        shape === "key" && "aspect-square rounded-[var(--key-radius)]",
        shape === "zone" && "aspect-[2/1] rounded-[calc(var(--key-radius)*0.6)]",
        shape === "dial" &&
          "aspect-square rounded-full border-2 border-device-dial-border bg-device-dial",
        mark,
        drag.isDragging && "opacity-40",
      )}
      onPointerDown={onPointerDown}
      onClick={(event) => props.onSelect?.(control, selectMode(event))}
      onDoubleClick={() => props.onOpen?.(control)}
      onKeyDown={(event) => {
        props.onControlKeyDown?.(event, control)
      }}
    >
      {shape === "dial" ? (
        <span className="flex size-full flex-col items-center justify-center rounded-full bg-device-dial p-[12%] text-center text-[length:max(9px,calc(var(--key)*0.12))] leading-tight text-device-dial-foreground">
          <span className="line-clamp-2">
            {(props.captions?.get(key) ?? "").split(" · ")[0]?.replace(/^[^:]*: /, "")}
          </span>
        </span>
      ) : image ? (
        // The key as the device draws it, edge to edge; the button carries the name.
        <img src={image} alt="" draggable={false} className="size-full object-cover" />
      ) : (
        <FaceTile
          face={
            unavailable === undefined
              ? face
              : { ...(face ?? { style: "action", title: "" }), detail: unavailable }
          }
          icon={props.icons?.get(key)}
          compact={shape === "zone"}
          unavailable={unavailable !== undefined}
        />
      )}
    </button>
  )
  return props.wrapControl ? props.wrapControl(control, button) : button
}

/** The CSS size of one key: `keySize` pixels, else the size step. */
function keyUnit(size: "sm" | "md", keySize: number | undefined): string {
  if (keySize !== undefined && keySize > 0) return `${keySize}px`
  return size === "sm" ? "2.25rem" : "4.25rem"
}

/**
 * A device drawn from its capabilities: keys, touch zones and dials. Every
 * measure is relative to the size of one key (`--key`), so the device keeps
 * its proportions at any `keySize` (see `device-geometry.ts`).
 */
export function DeviceCanvas(props: CanvasProps) {
  const { model, size = "md", onSelect, focused } = props
  const { keys, encoders, touchZones } = controlsOf(model)
  const geometry = deviceGeometry(model)
  const container = useRef<HTMLDivElement>(null)
  const unit = keyUnit(size, props.keySize)
  // The touch strip and the dials span the key grid, as on the hardware.
  const width = `calc(var(--key) * ${gridWidth(geometry)})`
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

  const gap = `calc(var(--key) * ${KEY_GAP})`
  return (
    <div
      ref={container}
      data-slot="device-canvas"
      data-model={model.id}
      className="inline-grid shrink-0 rounded-[calc(var(--key)*0.32)] border border-device-body-border bg-device-body shadow-sm"
      style={
        {
          "--key": unit,
          "--key-radius": `calc(var(--key) * 0.14)`,
          gap: `calc(var(--key) * ${KEY_GAP * 1.5})`,
          padding: `calc(var(--key) * ${BODY_PADDING})`,
        } as CSSProperties
      }
      role="group"
      aria-label={`${model.name} controls`}
      onKeyDown={onKeyDown}
    >
      {keys.length > 0 && (
        <div
          data-slot="device-keys"
          className="grid"
          style={{ gap, gridTemplateColumns: `repeat(${geometry.columns}, var(--key))` }}
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
          data-slot="device-strip"
          className="grid rounded-[var(--key-radius)] bg-device-strip"
          style={{
            width,
            gap: "calc(var(--key) * 0.1)",
            padding: "calc(var(--key) * 0.1)",
            gridTemplateColumns: `repeat(${touchZones.length}, minmax(0, 1fr))`,
          }}
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
          data-slot="device-dials"
          className="grid justify-items-center"
          style={{ width, gap, gridTemplateColumns: `repeat(${encoders.length}, minmax(0, 1fr))` }}
        >
          {encoders.map((control) => (
            <div key={controlKey(control)} style={{ width: `calc(var(--key) * ${DIAL_SIZE})` }}>
              <Control control={control} shape="dial" tabStop={isStop(control)} props={props} />
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
