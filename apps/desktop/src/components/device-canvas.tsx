import { useDraggable, useDroppable } from "@dnd-kit/core"
import {
  memo,
  useRef,
  type CSSProperties,
  type KeyboardEvent,
  type ReactElement,
  type ReactNode,
} from "react"

import {
  EDGE_ROOM,
  KEY_BORDER,
  keyBorderPixels,
  DIAL_SIZE,
  KEY_GAP,
  deviceGeometry,
  gridWidth,
} from "@/components/device-geometry"
import { KeyPreview } from "@/components/key-preview"
import { IS_MAC } from "@/hooks/use-navigation"
import { useTranslation } from "@/i18n"
import {
  neighbour,
  parseControlKey,
  type Direction,
  type DragSource,
  type SelectMode,
} from "@/layout-editor/editing"
import { controlKey, controlLabel, controlsOf, sameControl } from "@/lib/layout-model"
import type { ControlId, DeviceModel, KeyFace, MarqueePreview } from "@/lib/types"
import { cn } from "@/lib/utils"
import { inRange, type VisibleRange } from "@/layout-editor/canvas-viewport"

interface CanvasProps {
  model: DeviceModel
  faces?: Map<string, KeyFace>
  /** Rendered key images (PNG data URLs); shown instead of the drawn face. */
  images?: Map<string, string>
  /**
   * Keys whose title scrolls, with the frames the device shows: played in
   * place of their still image (layout-editor/use-marquee-frames.ts).
   */
  animations?: Map<string, MarqueePreview>
  /**
   * Keys that cannot do anything now, with why in a word or two (e.g. "No
   * previous layout"): drawn dimmed with that detail, as on the device.
   */
  unavailable?: Map<string, string>
  icons?: Map<string, string | null | undefined>
  /**
   * Protected system controls (a folder's Back key), with what they are in
   * words: drawn like any other key (no mark of their own), not draggable,
   * described with the text.
   */
  system?: Map<string, string>
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
  /**
   * The keys in view of a large grid (layout-editor/device-viewport.tsx):
   * the others are drawn as plain placeholders (no image or face) but keep
   * their button, so they stay in the accessibility tree, the focus order
   * and the drop targets. `null` or absent: every key is drawn.
   */
  visible?: VisibleRange | null
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
 * What every control shares: a dark LCD inside one border whose width
 * scales with the key (`--key-border`, set by the canvas). Every state —
 * selected, drop target, refused drop, hover — changes only the border's
 * colour, never its width, so a key keeps its size, radius and content
 * area in every state. Keyboard focus is an outline outside the key, which
 * does not take space either.
 */
const KEY_BASE = cn(
  "relative block w-full min-w-0 overflow-hidden border-solid border-(length:--key-border) bg-device-key",
  // The state's colour changes at once (no transition): selection is
  // immediate, also in a window WebKit throttles in the background.
  "transition-[filter] duration-100 disabled:cursor-default",
  "outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-device-focus focus-visible:outline-solid",
)

interface ControlProps {
  control: ControlId
  shape: "key" | "zone" | "dial"
  tabStop: boolean
  /** Outside the view of a large grid: drawn without its face. */
  culled?: boolean
  props: CanvasProps
}

/** The props a control draws from the maps for itself, compared by its entry. */
const PER_CONTROL = new Set([
  "faces",
  "images",
  "animations",
  "unavailable",
  "system",
  "icons",
  "captions",
  "summaries",
])
/** Props a control does not read (their effect comes as `tabStop` or `culled`), or reads as below. */
const NOT_READ = new Set([
  "model",
  "size",
  "keySize",
  "focused",
  "visible",
  "selection",
  "rejected",
])

/**
 * Whether a control would draw the same: what it reads of the canvas'
 * props for itself (its map entries, whether it is selected or refused)
 * and every other prop by identity. The key size is not among them (CSS
 * variables of the canvas), so zooming and panning do not draw the keys
 * again.
 */
function sameControlProps(a: ControlProps, b: ControlProps): boolean {
  if (
    a.shape !== b.shape ||
    a.tabStop !== b.tabStop ||
    a.culled !== b.culled ||
    !sameControl(a.control, b.control)
  )
    return false
  const key = controlKey(a.control)
  const p = a.props as unknown as Record<string, unknown>
  const q = b.props as unknown as Record<string, unknown>
  for (const name of new Set([...Object.keys(p), ...Object.keys(q)])) {
    if (NOT_READ.has(name)) continue
    if (PER_CONTROL.has(name)) {
      const left = p[name] as Map<string, unknown> | undefined
      const right = q[name] as Map<string, unknown> | undefined
      if (left?.get(key) !== right?.get(key)) return false
    } else if (p[name] !== q[name]) return false
  }
  const selected = (props: CanvasProps) =>
    props.selection?.some((c) => sameControl(c, a.control)) ?? false
  const rejected = (props: CanvasProps) =>
    props.rejected ? sameControl(props.rejected, a.control) : false
  return selected(a.props) === selected(b.props) && rejected(a.props) === rejected(b.props)
}

const Control = memo(function Control({ control, shape, tabStop, culled, props }: ControlProps) {
  // The control's name ("Key 1") is in the window's language.
  useTranslation()
  const key = controlKey(control)
  const drop = useDroppable({ id: key, disabled: !props.droppable })
  // What the editor's drop handler reads: without the type it took a
  // dragged control for a library entry.
  const source: DragSource = { type: "control", control }
  const system = props.system?.get(key)
  const drag = useDraggable({
    id: `control:${key}`,
    data: source,
    disabled: !props.draggable || system !== undefined,
  })
  const selected = props.selection?.some((c) => sameControl(c, control)) ?? false
  const interactive = Boolean(props.onSelect)
  const face = props.faces?.get(key)
  const unavailable = props.unavailable?.get(key)
  // Empty keys are drawn by the app (the key's own LCD tone, in either
  // theme) rather than as the device's black image.
  const image =
    unavailable === undefined && face?.style !== "empty" ? props.images?.get(key) : undefined
  const described = props.summaries?.get(key) ?? (shape === "dial" ? props.captions?.get(key) : "")
  const summary = unavailable && described ? `${described} (${unavailable})` : described
  const refusal =
    drop.isOver && props.dropRejection
      ? props.dropRejection(control, drop.active?.data.current)
      : null
  const refused =
    refusal !== null || (props.rejected ? sameControl(props.rejected, control) : false)
  // Only pointer dragging: Enter and Space keep selecting the control.
  const onPointerDown =
    props.draggable && system === undefined
      ? (drag.listeners?.onPointerDown as React.PointerEventHandler | undefined)
      : undefined
  // The state is the border's colour (same width in every state).
  const mark = refused
    ? "border-destructive"
    : drop.isOver
      ? "border-device-drop"
      : selected
        ? "border-device-selected"
        : "border-device-key-border enabled:hover:border-device-key-hover enabled:hover:brightness-110"
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
      data-system={system !== undefined ? "" : undefined}
      aria-description={system}
      disabled={!interactive}
      tabIndex={interactive ? (tabStop ? 0 : -1) : undefined}
      aria-label={controlLabel(control)}
      aria-pressed={interactive ? selected : undefined}
      title={
        system
          ? `${controlLabel(control)}: ${system}`
          : summary
            ? `${controlLabel(control)}: ${summary}`
            : controlLabel(control)
      }
      className={cn(
        KEY_BASE,
        shape === "key" && "aspect-square rounded-[var(--key-radius)]",
        shape === "zone" && "aspect-[2/1] rounded-[calc(var(--key-radius)*0.6)]",
        shape === "dial" && "aspect-square rounded-full bg-device-dial",
        mark,
        shape === "dial" && !refused && !drop.isOver && !selected && "border-device-dial-border",
        drag.isDragging && "opacity-40",
      )}
      onPointerDown={onPointerDown}
      onClick={(event) => props.onSelect?.(control, selectMode(event))}
      onDoubleClick={() => props.onOpen?.(control)}
      onKeyDown={(event) => {
        props.onControlKeyDown?.(event, control)
      }}
    >
      {culled ? null : shape === "dial" ? (
        <span className="flex size-full flex-col items-center justify-center rounded-full bg-device-dial p-[12%] text-center text-[length:max(9px,calc(var(--key)*0.12))] leading-tight text-device-dial-foreground">
          <span className="line-clamp-2">
            {(props.captions?.get(key) ?? "").split(" · ")[0]?.replace(/^[^:]*: /, "")}
          </span>
        </span>
      ) : (
        // The key as the device draws it, edge to edge (`KeyPreview`, also
        // the Inspector's); the button carries the name. Drawn at the size
        // it is shown at on this display (`keyPixels` of the preview), so
        // the browser does not scale it up.
        <KeyPreview
          face={face}
          image={image}
          animation={props.animations?.get(key)}
          icon={props.icons?.get(key)}
          compact={shape === "zone"}
          unavailable={unavailable}
        />
      )}
    </button>
  )
  return props.wrapControl ? props.wrapControl(control, button) : button
}, sameControlProps)

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
  const { t } = useTranslation()
  const { model, size = "md", onSelect, focused } = props
  const { keys, encoders, touchZones } = controlsOf(model)
  const geometry = deviceGeometry(model)
  const container = useRef<HTMLDivElement>(null)
  const unit = keyUnit(size, props.keySize)
  // The touch strip and the dials span the key grid, as on the hardware.
  const width =
    props.keySize !== undefined && props.keySize > 0
      ? `${geometry.columns * props.keySize + (geometry.columns - 1) * Math.round(props.keySize * KEY_GAP)}px`
      : `calc(var(--key) * ${gridWidth(geometry)})`
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

  // Whole pixels when the key size is known, so keys (and their images)
  // start on pixel boundaries instead of being resampled between them.
  const measure = (keys: number) =>
    props.keySize !== undefined && props.keySize > 0
      ? `${Math.round(props.keySize * keys)}px`
      : `calc(var(--key) * ${keys})`
  const gap = measure(KEY_GAP)
  const keyBorder =
    props.keySize !== undefined && props.keySize > 0
      ? `${keyBorderPixels(props.keySize)}px`
      : `max(2px, calc(var(--key) * ${KEY_BORDER}))`
  return (
    <div
      ref={container}
      data-slot="device-canvas"
      data-model={model.id}
      // The controls themselves, directly on the workspace: no enclosure.
      className="inline-grid shrink-0"
      style={
        {
          "--key": unit,
          "--key-radius": `calc(var(--key) * 0.14)`,
          // One border width for every control and state: about 5.5 % of
          // a key, in whole pixels so it stays sharp (never under 2px).
          "--key-border": keyBorder,
          gap: measure(KEY_GAP * 1.5),
          padding: measure(EDGE_ROOM),
        } as CSSProperties
      }
      role="group"
      aria-label={t("editor:canvas.controls", { model: model.name })}
      onKeyDown={onKeyDown}
    >
      {keys.length > 0 && (
        <div
          data-slot="device-keys"
          className="grid"
          style={{ gap, gridTemplateColumns: `repeat(${geometry.columns}, var(--key))` }}
        >
          {keys.map(({ control }, index) => (
            <Control
              key={controlKey(control)}
              control={control}
              shape="key"
              tabStop={isStop(control)}
              // The focused key is always drawn, wherever it is.
              culled={!inRange(props.visible ?? null, index, geometry.columns) && !isStop(control)}
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
