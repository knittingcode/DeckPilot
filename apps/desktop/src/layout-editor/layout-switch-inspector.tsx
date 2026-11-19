import { Check, ChevronsUpDown } from "lucide-react"
import { useId, useState } from "react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command"
import { Label } from "@/components/ui/label"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { api } from "@/lib/api"
import { failureDetail } from "@/lib/failure"
import {
  HISTORY_CHOICES,
  PREVIOUS_LAYOUT,
  SWITCH_LAYOUT,
  existingLayoutSwitch,
  layoutSwitchOf,
  layoutSwitchParameters,
  shownLayout,
  usedElsewhere,
  type LayoutHistory,
  type LayoutSwitch,
} from "@/lib/layout-navigation"
import { bindingsFor, controlKey, setBinding, triggerKey, triggerLabel } from "@/lib/layout-model"
import type {
  ActionInstance,
  ActionParameters,
  ControlBinding,
  ControlId,
  DeviceInfo,
  DeviceLayout,
  Usage,
} from "@/lib/types"
import { cn } from "@/lib/utils"

import { freeActionName } from "./catalog"

/** What the layout navigation panels need from the editor. */
export interface LayoutNavigationContext {
  profileId: string
  /** The model the layout is for, e.g. "Stream Deck MK.2". */
  modelName: string
  /** The profile's layouts for the model, in its order. */
  layouts: DeviceLayout[]
  /** The profile's default layout for the model. */
  defaultLayout: string | null
  /** Where the profile's actions are used (as saved). */
  usages: Record<string, Usage[]>
  /** The connected device the editor is for, if any. */
  device: DeviceInfo | null
  /** An action was created or changed: the editor shows it at once. */
  onActionSaved: (action: ActionInstance) => void
}

const SWITCH_KEYS = ["layout", "page", "history"]

/**
 * The Switch layout and Back to previous layout bindings of a control,
 * each with a compact panel of its own.
 */
export function LayoutNavigationPanels({
  draft,
  pageId,
  control,
  actions,
  context,
  onChange,
}: {
  draft: DeviceLayout
  pageId: string
  control: ControlId
  actions: ActionInstance[]
  context: LayoutNavigationContext
  onChange: (layout: DeviceLayout) => void
}) {
  const bindings = bindingsFor(draft, pageId, control).flatMap((binding) => {
    if (binding.target.type !== "action") return []
    const id = binding.target.action
    const action = actions.find((a) => a.id === id)
    if (action?.actionType !== SWITCH_LAYOUT && action?.actionType !== PREVIOUS_LAYOUT) return []
    return [{ binding, action }]
  })
  if (bindings.length === 0) return null
  const several = bindings.length > 1
  return (
    <>
      {bindings.map(({ binding, action }) => {
        const key = `${controlKey(control)}:${triggerKey(binding.trigger)}`
        const trigger = several ? triggerLabel(binding.trigger) : null
        return action.actionType === SWITCH_LAYOUT ? (
          <SwitchLayoutPanel
            key={key}
            draft={draft}
            pageId={pageId}
            binding={binding}
            action={action}
            actions={actions}
            context={context}
            trigger={trigger}
            onChange={onChange}
          />
        ) : (
          <PreviousLayoutPanel key={key} context={context} trigger={trigger} />
        )
      })}
    </>
  )
}

function PanelFrame({
  title,
  trigger,
  children,
}: {
  title: string
  trigger: string | null
  children: React.ReactNode
}) {
  const id = useId()
  return (
    <section className="grid gap-3 border-t pt-4" aria-labelledby={id}>
      <h3 id={id} className="text-sm font-medium">
        {title}
        {trigger && <span className="font-normal text-muted-foreground"> · {trigger}</span>}
      </h3>
      {children}
    </section>
  )
}

/** Where a Switch layout key goes: the layout, its page and what Back does then. */
function SwitchLayoutPanel({
  draft,
  pageId,
  binding,
  action,
  actions,
  context,
  trigger,
  onChange,
}: {
  draft: DeviceLayout
  pageId: string
  binding: ControlBinding
  action: ActionInstance
  actions: ActionInstance[]
  context: LayoutNavigationContext
  trigger: string | null
  onChange: (layout: DeviceLayout) => void
}) {
  const id = useId()
  const [busy, setBusy] = useState(false)
  const overrides = binding.target.type === "action" ? (binding.target.overrides ?? {}) : {}
  const current = layoutSwitchOf(action, overrides)
  const target = context.layouts.find((l) => l.id === current.layout)
  const shared = usedElsewhere(action.id, draft, pageId, binding, context.usages)

  /** Points this key at `next`, never changing what other keys do. */
  const change = async (next: LayoutSwitch) => {
    const name = (layoutId: string) => context.layouts.find((l) => l.id === layoutId)?.name
    const kept = Object.fromEntries(
      Object.entries(action.parameters).filter(([key]) => !SWITCH_KEYS.includes(key)),
    )
    const parameters: ActionParameters = { ...kept, ...layoutSwitchParameters(next) }
    setBusy(true)
    try {
      if (shared) {
        const reused = existingLayoutSwitch(actions, next)
        const chosen =
          reused && reused.id !== action.id
            ? reused
            : await api.createAction(context.profileId, {
                name: freeActionName(actions, name(next.layout) ?? "Switch layout"),
                actionType: SWITCH_LAYOUT,
                parameters,
                description: "",
                icon: action.icon ?? null,
                label: null,
              })
        if (chosen !== reused) context.onActionSaved(chosen)
        const rest = Object.fromEntries(
          Object.entries(overrides).filter(([key]) => !SWITCH_KEYS.includes(key)),
        )
        onChange(
          setBinding(draft, pageId, {
            ...binding,
            target: {
              type: "action",
              action: chosen.id,
              ...(Object.keys(rest).length > 0 ? { overrides: rest } : {}),
            },
          }),
        )
      } else {
        // Named after its layout: the name follows the new one.
        const before = name(current.layout)
        const after = name(next.layout)
        const renamed =
          before && after && action.name === before && before !== after
            ? freeActionName(
                actions.filter((a) => a.id !== action.id),
                after,
              )
            : action.name
        const updated = await api.updateAction(context.profileId, {
          ...action,
          name: renamed,
          parameters,
        })
        context.onActionSaved(updated)
      }
    } catch (e) {
      toast.error("The destination was not changed", { description: failureDetail(e) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <PanelFrame title="Switch layout" trigger={trigger}>
      <p className="text-sm" aria-live="polite">
        Shows:{" "}
        <span className="font-medium">
          {target?.name ?? (current.layout ? "a layout not in this profile" : "nothing chosen yet")}
        </span>
      </p>
      <div className="grid gap-1.5">
        <Label id={`${id}-layout`}>Layout</Label>
        <LayoutPicker
          labelledBy={`${id}-layout`}
          layouts={context.layouts}
          value={current.layout}
          defaultLayout={context.defaultLayout}
          disabled={busy}
          onSelect={(layout) => {
            if (layout !== current.layout) void change({ ...current, layout, page: null })
          }}
        />
        <p className="text-xs text-muted-foreground">
          Only layouts for {context.modelName} assigned to this profile.
        </p>
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor={`${id}-page`}>Page</Label>
        <Select
          value={current.page ?? "home"}
          disabled={busy || !target}
          onValueChange={(value) => {
            void change({ ...current, page: value === "home" ? null : value })
          }}
        >
          <SelectTrigger id={`${id}-page`} size="sm" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="home">Home page</SelectItem>
            {(target?.pages ?? [])
              .filter((p) => p.id !== target?.homePage)
              .map((p) => (
                <SelectItem key={p.id} value={p.id}>
                  {p.name}
                </SelectItem>
              ))}
            {current.page && !target?.pages.some((p) => p.id === current.page) && (
              <SelectItem value={current.page}>Missing page (home page)</SelectItem>
            )}
          </SelectContent>
        </Select>
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor={`${id}-history`}>Back</Label>
        <Select
          value={current.history}
          disabled={busy}
          onValueChange={(value) => {
            void change({ ...current, history: value as LayoutHistory })
          }}
        >
          <SelectTrigger id={`${id}-history`} size="sm" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {HISTORY_CHOICES.map((choice) => (
              <SelectItem key={choice.value} value={choice.value}>
                {choice.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground">
          {HISTORY_CHOICES.find((c) => c.value === current.history)?.detail}
        </p>
      </div>
      {shared && (
        <p className="text-xs text-muted-foreground" role="note">
          Other keys run “{action.name}” too: a change here gives this key an action of its own.
        </p>
      )}
    </PanelFrame>
  )
}

/** A searchable list of the layouts a Switch layout key can show. */
function LayoutPicker({
  labelledBy,
  layouts,
  value,
  defaultLayout,
  disabled,
  onSelect,
}: {
  labelledBy: string
  layouts: DeviceLayout[]
  value: string
  defaultLayout: string | null
  disabled: boolean
  onSelect: (layout: string) => void
}) {
  const [open, setOpen] = useState(false)
  const selected = layouts.find((l) => l.id === value)
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          role="combobox"
          aria-expanded={open}
          aria-labelledby={labelledBy}
          disabled={disabled}
          className="w-full justify-between font-normal"
        >
          <span className="truncate">{selected?.name ?? "Choose a layout"}</span>
          <ChevronsUpDown className="opacity-50" aria-hidden />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-(--radix-popover-trigger-width) min-w-56 p-0" align="start">
        <Command>
          <CommandInput placeholder="Search layouts" aria-label="Search layouts" />
          <CommandList>
            <CommandEmpty>No layout matches.</CommandEmpty>
            <CommandGroup>
              {layouts.map((layout) => (
                <CommandItem
                  key={layout.id}
                  value={`${layout.name} ${layout.id}`}
                  onSelect={() => {
                    setOpen(false)
                    onSelect(layout.id)
                  }}
                >
                  <Check
                    className={cn(layout.id === value ? "opacity-100" : "opacity-0")}
                    aria-hidden
                  />
                  <span className="min-w-0 flex-1 truncate">{layout.name}</span>
                  {layout.id === defaultLayout && (
                    <span className="text-xs text-muted-foreground">default</span>
                  )}
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

/** What Back to previous layout does, and whether the device can go back now. */
function PreviousLayoutPanel({
  context,
  trigger,
}: {
  context: LayoutNavigationContext
  trigger: string | null
}) {
  const { device } = context
  const previous = device?.previousLayout
  const connected = shownLayout(device) !== null
  return (
    <PanelFrame title="Back to previous layout" trigger={trigger}>
      <p className="text-xs text-muted-foreground">
        Returns this device to the layout it showed before, in this profile. Does nothing when there
        is none. A folder&apos;s Back key returns to a page instead.
      </p>
      {connected && (
        <p className="text-xs" role="status">
          {previous
            ? `Now: back to ${context.layouts.find((l) => l.id === previous)?.name ?? "the previous layout"}.`
            : "Now: this device has no previous layout, so the key does nothing."}
        </p>
      )}
    </PanelFrame>
  )
}
