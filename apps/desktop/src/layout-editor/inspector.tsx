import type { ReactNode } from "react"
import { Eraser, FolderOpen, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
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
  sameTrigger,
  setBinding,
  supportedTriggers,
  triggerKey,
  triggerLabel,
} from "@/lib/layout-model"
import type { ControlId, DeviceLayout, DeviceModel } from "@/lib/types"

import { decodeTarget, encodeTarget, type TargetGroup } from "./targets"

interface InspectorProps {
  model: DeviceModel
  draft: DeviceLayout
  pageId: string
  selection: ControlId[]
  focus: ControlId | null
  groups: TargetGroup[]
  folder: string | undefined
  /** Commits a change; `group` merges consecutive edits into one undo step. */
  onChange: (layout: DeviceLayout, group?: string) => void
  onClear: () => void
  onOpenFolder: () => void
  onClose: () => void
  /** The look of the selected keys (appearance panel). */
  appearance?: ReactNode
}

/** Settings of the selected control, or a summary of a multi-selection. */
export function Inspector(props: InspectorProps) {
  const { selection, focus } = props
  if (selection.length === 0 || !focus) {
    return (
      <Card className="h-fit">
        <CardHeader>
          <CardTitle className="text-sm">Control</CardTitle>
          <CardDescription>
            Select a key, dial or touch zone to configure it. Shift-click selects a range, arrow
            keys move between controls and Shift+Enter opens a folder.
          </CardDescription>
        </CardHeader>
      </Card>
    )
  }
  if (selection.length > 1) {
    return (
      <Card className="h-fit">
        <CardHeader className="flex flex-row items-center gap-2">
          <CardTitle className="flex-1 text-sm">{selection.length} controls selected</CardTitle>
          <Button variant="ghost" size="icon" aria-label="Clear selection" onClick={props.onClose}>
            <X />
          </Button>
        </CardHeader>
        <CardContent className="grid gap-3 text-sm">
          <p className="text-muted-foreground">
            {selection.map(controlLabel).join(", ")}. Copy, cut, paste, duplicate or clear them
            together, or drop a library entry onto one of them.
          </p>
          <Button variant="outline" size="sm" className="w-fit" onClick={props.onClear}>
            <Eraser /> Clear {selection.length} controls
          </Button>
          {props.appearance && <div className="border-t pt-4">{props.appearance}</div>}
        </CardContent>
      </Card>
    )
  }
  return <ControlInspector {...props} control={focus} />
}

function ControlInspector({
  model,
  draft,
  pageId,
  control,
  groups,
  folder,
  onChange,
  onClear,
  onOpenFolder,
  onClose,
  appearance,
}: InspectorProps & { control: ControlId }) {
  const bindings = bindingsFor(draft, pageId, control)
  return (
    <Card className="h-fit">
      <CardHeader className="flex flex-row items-center gap-2">
        <CardTitle className="flex-1 text-sm">{controlLabel(control)}</CardTitle>
        <Button variant="ghost" size="icon" aria-label="Clear selection" onClick={onClose}>
          <X />
        </Button>
      </CardHeader>
      <CardContent className="grid gap-4">
        {supportedTriggers(model, control).map((trigger) => {
          const binding = bindings.find((b) => sameTrigger(b.trigger, trigger))
          const id = `${controlKey(control)}-${triggerKey(trigger)}`
          return (
            <div key={triggerKey(trigger)} className="grid gap-2">
              <Label htmlFor={id}>{triggerLabel(trigger)}</Label>
              <Select
                value={encodeTarget(binding?.target)}
                onValueChange={(value) => {
                  const target = decodeTarget(value)
                  onChange(
                    target
                      ? setBinding(draft, pageId, {
                          control,
                          trigger,
                          target,
                          label: binding?.label ?? null,
                          ...(binding?.appearance ? { appearance: binding.appearance } : {}),
                        })
                      : removeBinding(draft, pageId, control, trigger),
                  )
                }}
              >
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
              {binding && (
                <Input
                  aria-label={`Label for ${triggerLabel(trigger)}`}
                  placeholder="Label (optional)"
                  maxLength={32}
                  value={binding.label ?? ""}
                  onChange={(e) => {
                    onChange(
                      setBinding(draft, pageId, { ...binding, label: e.target.value || null }),
                      `label:${id}`,
                    )
                  }}
                />
              )}
            </div>
          )
        })}
        <div className="flex flex-wrap gap-2">
          {folder && (
            <Button variant="outline" size="sm" onClick={onOpenFolder}>
              <FolderOpen /> Open folder page
            </Button>
          )}
          {bindings.length > 0 && (
            <Button variant="ghost" size="sm" onClick={onClear}>
              <Eraser /> Clear control
            </Button>
          )}
        </div>
        <p className="text-xs text-muted-foreground">
          Only events this model can produce are offered. Unbound controls stay blank on the device.
        </p>
        {appearance && <div className="border-t pt-4">{appearance}</div>}
      </CardContent>
    </Card>
  )
}
