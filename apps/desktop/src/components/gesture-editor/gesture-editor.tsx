import { useState } from "react"

import { ParameterField } from "@/components/parameter-field"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  bindingsFor,
  controlKey,
  controlLabel,
  removeBinding,
  setBinding,
  targetLabel,
} from "@/lib/layout-model"
import { initialValue } from "@/lib/parameters"
import type {
  ActionParameters,
  ActionType,
  BindingTarget,
  ControlBinding,
  ControlId,
  DeviceLayout,
  Gesture,
  GestureTimings,
  ParameterValue,
  Profile,
} from "@/lib/types"
import { setOverride } from "@/lib/workflow-model"

import {
  GESTURE_LABELS,
  GESTURES,
  TIMING_LIMITS,
  clickTiming,
  convertToGestures,
  gestureBinding,
  keyMode,
  keyTimings,
  needsConversion,
  setGesture,
  setKeyTiming,
  timingError,
  updateBinding,
  type ClickTiming,
  type TimingField,
} from "./model"

/** One option of the target selects; the layout editor's `TargetGroup`. */
export interface GestureTargetGroup {
  label: string
  options: { value: string; label: string; target: BindingTarget }[]
}

export interface GestureEditorProps {
  draft: DeviceLayout
  pageId: string
  control: ControlId
  profile: Pick<Profile, "actions" | "workflows" | "projects">
  /** Targets to offer, encoded with `encode`/`decode`. */
  groups: GestureTargetGroup[]
  /** Further targets only gestures can have (the Work Session controller, G4). */
  gestureGroups?: GestureTargetGroup[]
  encode: (target: BindingTarget | undefined) => string
  decode: (value: string) => BindingTarget | null
  /** Action types, for the parameters an override can replace. */
  types: ActionType[]
  /** This computer's default timings. */
  defaults: GestureTimings
  /** Messages of the last failed save about this control. */
  issues: string[]
  /** Commits a change; `group` merges consecutive edits into one undo step. */
  onChange: (layout: DeviceLayout, group?: string) => void
}

interface PendingConversion {
  gesture: Gesture
  target: BindingTarget
}

const CLICK_NOTES: Record<ClickTiming, (defaults: number) => string> = {
  immediate: () => "A single click fires as soon as the key is pressed.",
  onRelease: () =>
    "A single click fires when the key is released before the long-press time, so a hold can be told apart.",
  afterWindow: (ms) =>
    `A single click fires after the double-click window (${ms} ms) has passed without a second press.`,
}

/**
 * Click, double click and long press of one key or pressable dial, each
 * with its own target, parameter overrides and label, plus the key's
 * timings (docs/gestures-and-work-session.md, section 10).
 */
export function GestureEditor(props: GestureEditorProps) {
  const { draft, pageId, control, defaults, onChange } = props
  const [pending, setPending] = useState<PendingConversion | null>(null)
  const [overridesOpen, setOverridesOpen] = useState<Gesture | null>(null)
  const mode = keyMode(draft, pageId, control)
  const timing = clickTiming(draft, pageId, control)
  const own = keyTimings(draft, pageId, control)
  const release = bindingsFor(draft, pageId, control).find((b) => b.trigger.type === "release")
  const doubleClickMs = own?.doubleClickMs ?? defaults.doubleClickMs
  const base = `gesture-${controlKey(control)}`

  const assign = (gesture: Gesture, target: BindingTarget | null) => {
    if (target && needsConversion(draft, pageId, control, gesture, target)) {
      setPending({ gesture, target })
      return
    }
    onChange(setGesture(draft, pageId, control, gesture, target))
  }

  const confirmConversion = () => {
    if (!pending) return
    const converted = convertToGestures(draft, pageId, control).layout
    onChange(setGesture(converted, pageId, control, pending.gesture, pending.target))
    setPending(null)
  }

  const bound = (gesture: Gesture) =>
    bindingsFor(draft, pageId, control).some((b) => b.trigger.type === gesture)

  return (
    <section className="grid gap-4" aria-labelledby={`${base}-heading`}>
      <div className="grid gap-1">
        <h3 id={`${base}-heading`} className="text-sm font-medium">
          Gestures
        </h3>
        <p className="text-xs text-muted-foreground" role="note">
          {mode === "raw"
            ? "Single click (immediate): this key acts as soon as it is pressed. Adding a double click or long press switches it to gestures."
            : CLICK_NOTES[timing](doubleClickMs)}
        </p>
      </div>

      {GESTURES.map((gesture) => (
        <GestureRow
          key={gesture}
          {...props}
          gesture={gesture}
          label={
            gesture === "click" && mode !== "gesture"
              ? `${GESTURE_LABELS.click} (immediate)`
              : GESTURE_LABELS[gesture]
          }
          id={`${base}-${gesture}`}
          binding={gestureBinding(draft, pageId, control, gesture)}
          overridesOpen={overridesOpen === gesture}
          onToggleOverrides={() => {
            setOverridesOpen(overridesOpen === gesture ? null : gesture)
          }}
          onAssign={(target) => {
            assign(gesture, target)
          }}
        />
      ))}

      {mode !== "gesture" && (
        <div className="grid gap-2" role="group" aria-labelledby={`${base}-release-label`}>
          <Label id={`${base}-release-label`} htmlFor={`${base}-release`}>
            Release (immediate)
          </Label>
          <TargetSelect
            id={`${base}-release`}
            groups={props.groups}
            encode={props.encode}
            value={release?.target}
            onValueChange={(value) => {
              const target = props.decode(value)
              onChange(
                target
                  ? setBinding(draft, pageId, {
                      control,
                      trigger: { type: "release" },
                      target,
                      label: release?.label ?? null,
                      ...(release?.appearance ? { appearance: release.appearance } : {}),
                    })
                  : removeBinding(draft, pageId, control, { type: "release" }),
              )
            }}
          />
          <p className="text-xs text-muted-foreground">
            Runs when the key is let go, without delay. Keys with a double click or long press
            cannot use it.
          </p>
        </div>
      )}

      {(bound("doubleClick") || own?.doubleClickMs !== undefined) && (
        <TimingInput
          id={`${base}-double-ms`}
          field="doubleClickMs"
          value={own?.doubleClickMs}
          fallback={defaults.doubleClickMs}
          onCommit={(value) => {
            onChange(
              setKeyTiming(draft, pageId, control, "doubleClickMs", value),
              `${base}-double-ms`,
            )
          }}
        />
      )}
      {(bound("longPress") || own?.longPressMs !== undefined) && (
        <TimingInput
          id={`${base}-long-ms`}
          field="longPressMs"
          value={own?.longPressMs}
          fallback={defaults.longPressMs}
          onCommit={(value) => {
            onChange(setKeyTiming(draft, pageId, control, "longPressMs", value), `${base}-long-ms`)
          }}
        />
      )}

      {props.issues.length > 0 && (
        <ul className="grid gap-1 text-xs text-destructive" role="alert">
          {props.issues.map((issue) => (
            <li key={issue}>{issue}</li>
          ))}
        </ul>
      )}

      <ConversionDialog
        control={control}
        pending={pending}
        release={release}
        doubleClickMs={doubleClickMs}
        longPressMs={own?.longPressMs ?? defaults.longPressMs}
        draft={draft}
        profile={props.profile}
        onConfirm={confirmConversion}
        onCancel={() => {
          setPending(null)
        }}
      />
    </section>
  )
}

function TargetSelect({
  id,
  groups,
  encode,
  value,
  onValueChange,
}: {
  id: string
  groups: GestureTargetGroup[]
  encode: (target: BindingTarget | undefined) => string
  value: BindingTarget | undefined
  onValueChange: (value: string) => void
}) {
  return (
    <Select value={encode(value)} onValueChange={onValueChange}>
      <SelectTrigger id={id} className="w-full">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="none">Nothing</SelectItem>
        {groups.map((group) => (
          <SelectGroup key={group.label}>
            <SelectLabel>{group.label}</SelectLabel>
            {group.options.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectGroup>
        ))}
      </SelectContent>
    </Select>
  )
}

function GestureRow({
  draft,
  pageId,
  profile,
  groups,
  gestureGroups,
  encode,
  decode,
  types,
  gesture,
  label,
  id,
  binding,
  overridesOpen,
  onToggleOverrides,
  onAssign,
  onChange,
}: GestureEditorProps & {
  gesture: Gesture
  label: string
  id: string
  binding: ControlBinding | undefined
  overridesOpen: boolean
  onToggleOverrides: () => void
  onAssign: (target: BindingTarget | null) => void
}) {
  const target = binding?.target
  const action =
    target?.type === "action" ? profile.actions.find((a) => a.id === target.action) : undefined
  const type = action ? types.find((t) => t.id === action.actionType) : undefined
  const overrides = target?.type === "action" ? (target.overrides ?? {}) : {}
  const overrideCount = Object.keys(overrides).length

  const setOverrides = (next: ActionParameters) => {
    if (!binding || binding.target.type !== "action") return
    const updated: BindingTarget = {
      type: "action",
      action: binding.target.action,
      ...(Object.keys(next).length > 0 ? { overrides: next } : {}),
    }
    onChange(updateBinding(draft, pageId, { ...binding, target: updated }), `${id}-overrides`)
  }

  return (
    <div className="grid gap-2" role="group" aria-labelledby={`${id}-label`}>
      <Label id={`${id}-label`} htmlFor={id}>
        {label}
      </Label>
      <TargetSelect
        id={id}
        groups={[...groups, ...(gestureGroups ?? [])]}
        encode={encode}
        value={target}
        onValueChange={(value) => {
          onAssign(decode(value))
        }}
      />
      {binding && (
        <Input
          aria-label={`Label for ${label}`}
          placeholder="Label (optional)"
          maxLength={32}
          value={binding.label ?? ""}
          onChange={(e) => {
            onChange(
              updateBinding(draft, pageId, { ...binding, label: e.target.value || null }),
              `${id}-label`,
            )
          }}
        />
      )}
      {action && type && type.parameters.length > 0 && (
        <>
          <Button
            variant="link"
            size="sm"
            className="h-auto w-fit p-0"
            aria-expanded={overridesOpen}
            aria-controls={`${id}-overrides`}
            onClick={onToggleOverrides}
          >
            {overridesOpen
              ? "Hide parameters"
              : overrideCount > 0
                ? `Parameters for this gesture (${overrideCount} overridden)`
                : "Override parameters for this gesture"}
          </Button>
          {overridesOpen && (
            <div id={`${id}-overrides`} className="grid gap-2">
              <p className="text-xs text-muted-foreground">
                Overrides apply to this {GESTURE_LABELS[gesture].toLowerCase()} only; “{action.name}
                ” itself stays unchanged.
              </p>
              {type.parameters.map((spec) => {
                const value: ParameterValue | undefined = overrides[spec.key]
                return (
                  <div key={spec.key} className="grid gap-2 rounded-md border border-dashed p-2">
                    <label className="flex items-center gap-2 text-sm">
                      <Checkbox
                        checked={value !== undefined}
                        onCheckedChange={(checked) => {
                          setOverrides(
                            setOverride(
                              overrides,
                              spec.key,
                              checked === true
                                ? (action.parameters[spec.key] ?? initialValue(spec))
                                : undefined,
                            ),
                          )
                        }}
                      />
                      Override “{spec.label}”
                    </label>
                    {value !== undefined && (
                      <ParameterField
                        spec={spec}
                        value={value}
                        projects={profile.projects}
                        onChange={(next) => {
                          setOverrides(setOverride(overrides, spec.key, next))
                        }}
                      />
                    )}
                  </div>
                )
              })}
            </div>
          )}
        </>
      )}
    </div>
  )
}

/**
 * A timing of the key in milliseconds. Empty uses the default. Values out of
 * range are explained and not applied.
 */
function TimingInput({
  id,
  field,
  value,
  fallback,
  onCommit,
}: {
  id: string
  field: TimingField
  value: number | undefined
  fallback: number
  onCommit: (value: number | undefined) => void
}) {
  const { min, max, label } = TIMING_LIMITS[field]
  const [text, setText] = useState(value === undefined ? "" : String(value))
  // Follow changes made elsewhere (undo, "Use default"), not the typing.
  const [shown, setShown] = useState(value)
  if (shown !== value) {
    setShown(value)
    setText(value === undefined ? "" : String(value))
  }
  const error = timingError(field, text)
  return (
    <div className="grid gap-1">
      <Label htmlFor={id}>{label} (ms)</Label>
      <div className="flex items-center gap-2">
        <Input
          id={id}
          type="number"
          inputMode="numeric"
          min={min}
          max={max}
          step={10}
          className="w-32"
          placeholder={`Default (${fallback})`}
          value={text}
          aria-invalid={error ? true : undefined}
          aria-describedby={`${id}-hint${error ? ` ${id}-error` : ""}`}
          onChange={(e) => {
            const next = e.target.value
            setText(next)
            if (timingError(field, next) === null) {
              onCommit(next.trim() === "" ? undefined : Number(next))
            }
          }}
        />
        {value !== undefined && (
          <Button
            variant="ghost"
            size="sm"
            aria-label={`Use the default ${label.toLowerCase()}`}
            onClick={() => {
              setText("")
              onCommit(undefined)
            }}
          >
            Use default
          </Button>
        )}
      </div>
      <p id={`${id}-hint`} className="text-xs text-muted-foreground">
        {min}–{max} ms. {value === undefined ? "Using" : "Default:"} this computer&apos;s default (
        {fallback} ms).
      </p>
      {error && (
        <p id={`${id}-error`} className="text-xs text-destructive" role="alert">
          {error}
        </p>
      )}
    </div>
  )
}

function ConversionDialog({
  control,
  pending,
  release,
  doubleClickMs,
  longPressMs,
  draft,
  profile,
  onConfirm,
  onCancel,
}: {
  control: ControlId
  pending: PendingConversion | null
  release: ControlBinding | undefined
  doubleClickMs: number
  longPressMs: number
  draft: DeviceLayout
  profile: GestureEditorProps["profile"]
  onConfirm: () => void
  onCancel: () => void
}) {
  const name = controlLabel(control)
  const adding = pending ? GESTURE_LABELS[pending.gesture].toLowerCase() : ""
  const after =
    pending?.gesture === "doubleClick"
      ? `after the double-click window (${doubleClickMs} ms) has passed without a second press`
      : `when the key is released before the long-press time (${longPressMs} ms)`
  return (
    <AlertDialog
      open={pending !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Switch {name} to gestures?</AlertDialogTitle>
          <AlertDialogDescription asChild>
            <div className="grid gap-2">
              {pending?.gesture === "click" ? (
                <p>
                  {name} acts as soon as it is pressed. A Work Session controller needs gestures, so
                  its single click becomes a gesture. With no double click or long press it still
                  fires on the press.
                </p>
              ) : (
                <p>
                  {name} acts as soon as it is pressed. To add a {adding}, its single click becomes
                  a gesture: it then fires {after}, so it reacts a little later than now.
                </p>
              )}
              {release && (
                <p>
                  Its release binding (“
                  {targetLabel(release.target, profile.actions, profile.workflows, draft)}”) is
                  removed: a key with gestures cannot also react to its release.
                </p>
              )}
              <p>Nothing changes on the device until you save the layout.</p>
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction onClick={onConfirm}>Switch to gestures</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
