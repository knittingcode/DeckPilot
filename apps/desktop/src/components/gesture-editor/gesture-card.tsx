import { useDroppable } from "@dnd-kit/core"
import { CircleHelp, HandGrab, ListOrdered, MousePointerClick, X } from "lucide-react"
import { useState } from "react"

import { IncompatibleNote } from "@/components/incompatible-note"
import { ParameterField } from "@/components/parameter-field"
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
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { useTranslation } from "@/i18n"
import { initialValue } from "@/lib/parameters"
import type {
  ActionParameters,
  ActionType,
  BindingTarget,
  ControlBinding,
  DeviceLayout,
  Profile,
  ParameterValue,
  Trigger,
} from "@/lib/types"
import { cn } from "@/lib/utils"
import { setOverride } from "@/lib/workflow-model"

import { TIMING_LIMITS, timingError, updateBinding, type TimingField } from "./model"

/** One option of the target selects; the layout editor's `TargetGroup`. */
export interface GestureTargetGroup {
  label: string
  options: { value: string; label: string; target: BindingTarget; unavailable?: string }[]
}

/** Why the option of `value` cannot be placed now, if it cannot. */
function unavailableIn(groups: GestureTargetGroup[], value: string): string | undefined {
  for (const group of groups) {
    const found = group.options.find((o) => o.value === value)
    if (found) return found.unavailable
  }
  return undefined
}

/** Why a target already assigned cannot run now; it stays assigned. */
function UnavailableNote({ reason }: { reason: string | undefined }) {
  const { t } = useTranslation()
  if (!reason) return null
  return (
    <p className="text-xs text-destructive" role="note">
      {t("editor:gesture.unavailable", { reason })}
    </p>
  )
}

/** A timing of the key shown in a gesture's card. */
export interface CardTiming {
  id: string
  field: TimingField
  value: number | undefined
  fallback: number
  onCommit: (value: number | undefined) => void
}

export interface GestureCardProps {
  /** The base of the card's element ids. */
  id: string
  /** The gesture's name, e.g. "Double click" or "Single press". */
  title: string
  /** What the gesture (or the raw release) is bound as now. */
  trigger: Trigger
  binding: ControlBinding | undefined
  /** Every target the chooser offers. */
  groups: GestureTargetGroup[]
  encode: (target: BindingTarget | undefined) => string
  decode: (value: string) => BindingTarget | null
  /** Assigns the chosen target; `null` removes the assignment. */
  onAssign: (target: BindingTarget | null) => void
  /** Why `target` cannot run from this control for `trigger`; `null` when it can. */
  incompatible?: (target: BindingTarget, trigger: Trigger) => string | null
  /** Why the binding is kept but not run, if it is. */
  notRun?: string
  /**
   * The card's drop target id (gestures in the layout editor's drag and
   * drop); `droppable`: it takes drops now (its card is in view).
   */
  slot?: string
  droppable?: boolean
  /** The page shown, for refusing a drop of a page onto itself. */
  pageId?: string
  /** What the gesture does, behind a help icon next to its name. */
  help?: string
  /** The key's timing this gesture uses, edited here. */
  timing?: CardTiming
  /**
   * Edits of the assignment itself (its label, parameter overrides, the
   * steps of its Multi Action); without it the card only chooses.
   */
  details?: {
    draft: DeviceLayout
    pageId: string
    profile: Pick<Profile, "actions" | "workflows" | "projects">
    types: ActionType[]
    /** The gesture's name inside a sentence, e.g. "double click". */
    inSentence: string
    onChange: (layout: DeviceLayout, group?: string) => void
    onEditWorkflow?: (workflow: string) => void
    labelPlaceholder?: (binding: ControlBinding) => string | undefined
    /** The label is the key's title, edited in the key header instead. */
    titleInHeader?: (binding: ControlBinding) => boolean
  }
}

/**
 * One gesture of a key as a card: its name, what it does (or a compact
 * "Add action" when nothing), and the assignment's details. The same card
 * for every gesture, and for the raw release.
 */
export function GestureCard(props: GestureCardProps) {
  const { t } = useTranslation()
  const { id, title, binding, slot } = props
  const { setNodeRef, isOver, active } = useDroppable({
    id: slot ?? `${id}-card`,
    disabled: slot === undefined || props.droppable !== true,
  })
  const notes = binding ? unavailableIn(props.groups, props.encode(binding.target)) : undefined
  const drop = isOver
    ? dropRefused(active?.data.current, props.trigger, props.incompatible, props.pageId)
      ? "rejected"
      : "accepted"
    : undefined
  return (
    <div
      ref={setNodeRef}
      role="group"
      aria-labelledby={`${id}-label`}
      data-slot="gesture-card"
      data-gesture-slot={slot === undefined ? undefined : binding ? "filled" : "empty"}
      data-drop={drop}
      className={cn(
        // The state is the border's colour and style, never its width.
        "flex min-w-0 flex-1 flex-col gap-2 rounded-lg border bg-card p-3 text-card-foreground transition-[border-color,background-color,box-shadow]",
        !binding && "border-dashed",
        // The empty card's chooser fills its body; the card shows its focus.
        "has-[[data-slot=gesture-card-empty]:focus-visible]:border-ring has-[[data-slot=gesture-card-empty]:focus-visible]:ring-[3px] has-[[data-slot=gesture-card-empty]:focus-visible]:ring-ring/50",
        drop === "accepted" && "border-workspace-selection bg-workspace-selection/5",
        drop === "rejected" && "border-destructive",
      )}
    >
      <div className="flex min-h-6 items-start gap-2">
        <div className="flex min-w-0 flex-1 items-start gap-1.5">
          <Label
            id={`${id}-label`}
            htmlFor={id}
            // Long names (Polish) wrap within the header.
            className="min-w-0 pt-0.5 leading-snug break-words"
          >
            {title}
          </Label>
          {props.help && <HelpTip title={title} text={props.help} />}
        </div>
        {binding && (
          <Button
            variant="ghost"
            size="icon-xs"
            className="-mr-1"
            aria-label={t("editor:gesture.remove", { gesture: title })}
            title={t("editor:gesture.removeTitle", { gesture: title.toLowerCase() })}
            onClick={() => {
              props.onAssign(null)
            }}
          >
            <X />
          </Button>
        )}
      </div>
      <TargetSelect
        id={id}
        groups={props.groups}
        encode={props.encode}
        trigger={props.trigger}
        incompatible={props.incompatible}
        value={binding?.target}
        title={title}
        droppable={slot !== undefined}
        onValueChange={(value) => {
          props.onAssign(props.decode(value))
        }}
      />
      <UnavailableNote reason={notes} />
      <IncompatibleNote reason={binding ? props.notRun : undefined} />
      {binding && props.details && (
        <AssignmentDetails id={id} title={title} binding={binding} details={props.details} />
      )}
      {props.timing && <TimingInput key={props.timing.id} {...props.timing} />}
    </div>
  )
}

/**
 * A help icon next to a gesture's name: its tooltip (in a portal, so the
 * cards' clipping never cuts it) opens on hover and on keyboard focus, and
 * a click or tap toggles it.
 */
function HelpTip({ title, text }: { title: string; text: string }) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <TooltipProvider delayDuration={200}>
      <Tooltip open={open} onOpenChange={setOpen}>
        <TooltipTrigger asChild>
          <button
            type="button"
            data-slot="gesture-card-help"
            aria-label={t("editor:gesture.help", { gesture: title })}
            className="mt-0.5 inline-flex size-5 shrink-0 items-center justify-center rounded-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
            onClick={(event) => {
              // A click (or a tap, which opens no tooltip) toggles it.
              event.preventDefault()
              setOpen(!open)
            }}
          >
            <CircleHelp aria-hidden className="size-3.5" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="top" className="max-w-72">
          {text}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}

/**
 * Whether what is dragged over a card is refused there: anything but an
 * action type, a new Multi Action or a target (an action, a workflow, a
 * page other than this one), or a target that cannot run from this
 * gesture. Dropping it is refused by the editor's drop path, as before.
 */
function dropRefused(
  dragged: unknown,
  trigger: Trigger,
  incompatible: GestureCardProps["incompatible"],
  pageId: string | undefined,
): boolean {
  const source = dragged as { type?: string; target?: BindingTarget } | undefined
  if (source?.type === "actionType" || source?.type === "newMultiAction") return false
  if (source?.type !== "target" || !source.target) return true
  if (source.target.type === "openPage" && source.target.page === pageId) return true
  return Boolean(incompatible?.(source.target, trigger))
}

/**
 * The chooser of what a gesture does. With nothing assigned it fills the
 * card's body as the drop hint, and opens the same list.
 */
function TargetSelect({
  id,
  groups,
  encode,
  trigger,
  incompatible,
  value,
  title,
  droppable,
  onValueChange,
}: {
  id: string
  /** The gesture's name, for the empty chooser's name. */
  title: string
  /** The card takes drops: its empty body says so. */
  droppable: boolean
  groups: GestureTargetGroup[]
  encode: (target: BindingTarget | undefined) => string
  /** What the chosen target is bound to, for `incompatible`. */
  trigger: Trigger
  incompatible: GestureCardProps["incompatible"]
  value: BindingTarget | undefined
  onValueChange: (value: string) => void
}) {
  const { t } = useTranslation()
  const empty = value === undefined
  return (
    <Select
      value={empty ? "" : encode(value)}
      onValueChange={(next) => {
        // Only a choice: not the placeholder's empty value, not the value
        // it already has (a programmatic change must never assign).
        if (next === "" || next === (empty ? "" : encode(value))) return
        onValueChange(next)
      }}
    >
      {empty ? (
        <SelectTrigger
          id={id}
          data-slot="gesture-card-empty"
          aria-label={t("editor:gesture.empty.assign", { gesture: title })}
          aria-description={t(
            droppable
              ? "editor:gesture.empty.description"
              : "editor:gesture.empty.chooseDescription",
          )}
          className="min-h-16 w-full flex-1 justify-center rounded-md border-0 bg-transparent p-2 whitespace-normal shadow-none outline-none hover:bg-transparent focus-visible:ring-0 data-[size=default]:h-auto dark:bg-transparent dark:hover:bg-transparent [&>svg:last-child]:hidden"
        >
          <span
            data-slot="gesture-card-empty-content"
            className="flex flex-col items-center gap-1.5 text-center text-xs text-muted-foreground"
          >
            {droppable ? (
              <HandGrab aria-hidden className="size-5 opacity-70" />
            ) : (
              <MousePointerClick aria-hidden className="size-5 opacity-70" />
            )}
            {t(droppable ? "editor:gesture.empty.drop" : "editor:gesture.empty.choose")}
          </span>
        </SelectTrigger>
      ) : (
        <SelectTrigger id={id} className="w-full min-w-0">
          <SelectValue />
        </SelectTrigger>
      )}
      <SelectContent>
        <SelectItem value="none">{t("editor:gesture.nothing")}</SelectItem>
        {groups.map((group) => (
          <SelectGroup key={group.label}>
            <SelectLabel>{group.label}</SelectLabel>
            {group.options.map((option) => {
              const why = incompatible?.(option.target, trigger) ?? undefined
              return (
                <SelectItem
                  key={option.value}
                  value={option.value}
                  disabled={option.unavailable !== undefined || why !== undefined}
                  title={why}
                >
                  {option.label}
                  {option.unavailable !== undefined
                    ? ` ${t("editor:gesture.optionUnavailable")}`
                    : why !== undefined && ` ${t("editor:gesture.optionNotHere")}`}
                </SelectItem>
              )
            })}
          </SelectGroup>
        ))}
      </SelectContent>
    </Select>
  )
}

/** The label, steps and parameter overrides of an assigned gesture. */
function AssignmentDetails({
  id,
  title,
  binding,
  details,
}: {
  id: string
  title: string
  binding: ControlBinding
  details: NonNullable<GestureCardProps["details"]>
}) {
  const { t } = useTranslation()
  const [overridesOpen, setOverridesOpen] = useState(false)
  const { draft, pageId, profile, onChange } = details
  const target = binding.target
  const action =
    target.type === "action" ? profile.actions.find((a) => a.id === target.action) : undefined
  const type = action ? details.types.find((c) => c.id === action.actionType) : undefined
  const overrides = target.type === "action" ? (target.overrides ?? {}) : {}
  const overrideCount = Object.keys(overrides).length
  // A step workflow is edited from the key; a graph in the workflow builder.
  const steps =
    target.type === "workflow"
      ? profile.workflows.find((w) => w.id === target.workflow && !w.graph)
      : undefined

  const setOverrides = (next: ActionParameters) => {
    if (binding.target.type !== "action") return
    const updated: BindingTarget = {
      type: "action",
      action: binding.target.action,
      ...(Object.keys(next).length > 0 ? { overrides: next } : {}),
    }
    onChange(updateBinding(draft, pageId, { ...binding, target: updated }), `${id}-overrides`)
  }

  return (
    <>
      {!details.titleInHeader?.(binding) && (
        <Input
          aria-label={t("editor:gesture.labelFor", { gesture: title })}
          placeholder={details.labelPlaceholder?.(binding) ?? t("editor:gesture.labelPlaceholder")}
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
      {steps && details.onEditWorkflow && (
        <Button
          variant="outline"
          size="sm"
          className="w-fit max-w-full"
          onClick={() => {
            details.onEditWorkflow?.(steps.id)
          }}
        >
          <ListOrdered /> {t("editor:gesture.editSteps", { count: steps.steps.length })}
        </Button>
      )}
      {action && type && type.parameters.length > 0 && (
        <>
          <Button
            variant="link"
            size="sm"
            className="h-auto w-fit max-w-full p-0 text-left whitespace-normal"
            aria-expanded={overridesOpen}
            aria-controls={`${id}-overrides`}
            onClick={() => {
              setOverridesOpen(!overridesOpen)
            }}
          >
            {overridesOpen
              ? t("editor:gesture.overrides.hide")
              : overrideCount > 0
                ? t("editor:gesture.overrides.some", { count: overrideCount })
                : t("editor:gesture.overrides.none")}
          </Button>
          {overridesOpen && (
            <div id={`${id}-overrides`} className="grid min-w-0 gap-2">
              <p className="text-xs text-muted-foreground">
                {t("editor:gesture.overrides.hint", {
                  gesture: details.inSentence,
                  action: action.name,
                })}
              </p>
              {type.parameters.map((spec) => {
                const value: ParameterValue | undefined = overrides[spec.key]
                return (
                  <div key={spec.key} className="grid min-w-0 gap-2 rounded-md bg-muted/40 p-2">
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
                      {t("editor:gesture.overrides.override", { label: spec.label })}
                    </label>
                    {value !== undefined && (
                      <ParameterField
                        spec={spec}
                        value={value}
                        projects={profile.projects}
                        actionType={action.actionType}
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
    </>
  )
}

/**
 * A timing of the key in milliseconds. Empty uses the default. Values out of
 * range are explained and not applied.
 */
function TimingInput({ id, field, value, fallback, onCommit }: CardTiming) {
  const { t } = useTranslation()
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
    <div className="mt-auto grid gap-1 border-t pt-2">
      <Label htmlFor={id} className="leading-snug">
        {t("editor:gesture.timing.label", { label })}
      </Label>
      <div className="flex flex-wrap items-center gap-2">
        <Input
          id={id}
          type="number"
          inputMode="numeric"
          min={min}
          max={max}
          step={10}
          className="w-28"
          placeholder={t("editor:gesture.timing.placeholder", { ms: fallback })}
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
            aria-label={t("editor:gesture.timing.useDefaultLabel", { label: label.toLowerCase() })}
            onClick={() => {
              setText("")
              onCommit(undefined)
            }}
          >
            {t("editor:gesture.timing.useDefault")}
          </Button>
        )}
      </div>
      <p id={`${id}-hint`} className="text-xs text-muted-foreground">
        {value === undefined
          ? t("editor:gesture.timing.usingDefault", { min, max, ms: fallback })
          : t("editor:gesture.timing.default", { min, max, ms: fallback })}
      </p>
      {error && (
        <p id={`${id}-error`} className="text-xs text-destructive" role="alert">
          {error}
        </p>
      )}
    </div>
  )
}
