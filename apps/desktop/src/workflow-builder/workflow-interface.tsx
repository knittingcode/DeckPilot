import { Plus, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { ValueKind, WorkflowInput, WorkflowOutput } from "@/lib/types"

import { MAX_WORKFLOW_VALUES } from "./graph"
import { VALUE_KIND_LABEL } from "./mapping"

const KINDS = Object.keys(VALUE_KIND_LABEL) as ValueKind[]

/** Whether `key` is a valid input or output key (as the engine checks it). */
function isValueKey(key: string) {
  return /^[A-Za-z0-9_]{1,64}$/.test(key)
}

/** An unused key `<base>_<n>`. */
function nextKey(base: string, taken: { key: string }[]) {
  for (let n = 1; ; n++) {
    const key = `${base}_${n}`
    if (!taken.some((v) => v.key === key)) return key
  }
}

function Check({
  id,
  label,
  checked,
  onChange,
}: {
  id: string
  label: string
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <div className="flex items-center gap-1.5">
      <Checkbox
        id={id}
        checked={checked}
        onCheckedChange={(value) => {
          onChange(value === true)
        }}
      />
      <Label htmlFor={id} className="text-xs font-normal">
        {label}
      </Label>
    </div>
  )
}

function KindSelect({
  value,
  label,
  onChange,
}: {
  value: ValueKind
  label: string
  onChange: (kind: ValueKind) => void
}) {
  return (
    <Select
      value={value}
      onValueChange={(kind) => {
        onChange(kind as ValueKind)
      }}
    >
      <SelectTrigger className="h-7 w-28 text-xs" aria-label={label}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {KINDS.map((kind) => (
          <SelectItem key={kind} value={kind}>
            {VALUE_KIND_LABEL[kind]}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

function KeyField({
  value,
  label,
  onChange,
}: {
  value: string
  label: string
  onChange: (key: string) => void
}) {
  return (
    <Input
      aria-label={label}
      aria-invalid={!isValueKey(value)}
      className="h-7 min-w-0 flex-1 font-mono text-xs"
      value={value}
      onChange={(e) => {
        onChange(e.target.value)
      }}
    />
  )
}

/**
 * The open workflow's interface: typed inputs a Sub-workflow node gives it
 * and outputs it returns. The engine validates them with the graph.
 */
export function WorkflowInterfaceEditor({
  inputs,
  outputs,
  onInputs,
  onOutputs,
}: {
  inputs: WorkflowInput[]
  outputs: WorkflowOutput[]
  onInputs: (inputs: WorkflowInput[]) => void
  onOutputs: (outputs: WorkflowOutput[]) => void
}) {
  const setInput = (index: number, change: Partial<WorkflowInput>) => {
    onInputs(inputs.map((input, i) => (i === index ? { ...input, ...change } : input)))
  }
  const setOutput = (index: number, change: Partial<WorkflowOutput>) => {
    onOutputs(outputs.map((output, i) => (i === index ? { ...output, ...change } : output)))
  }
  return (
    <section className="grid gap-3 border-t px-4 py-3" aria-label="Inputs and outputs">
      <div className="grid gap-1">
        <h3 className="text-xs font-medium text-muted-foreground uppercase">Inputs</h3>
        <p className="text-xs text-muted-foreground">
          Values a Sub-workflow node gives this workflow. Nodes read them from the start node.
        </p>
      </div>
      {inputs.map((input, index) => (
        <div
          key={index}
          className="grid grid-cols-[minmax(0,1fr)] gap-1.5 rounded-md border p-2"
          role="group"
          aria-label={`Input ${input.key}`}
        >
          <div className="flex items-center gap-1">
            <KeyField
              label="Input key"
              value={input.key}
              onChange={(key) => {
                setInput(index, { key })
              }}
            />
            <KindSelect
              label="Input type"
              value={input.kind}
              onChange={(kind) => {
                setInput(index, { kind })
              }}
            />
            <Button
              variant="ghost"
              size="icon"
              className="size-7 shrink-0"
              aria-label={`Remove input ${input.key}`}
              onClick={() => {
                onInputs(inputs.filter((_, i) => i !== index))
              }}
            >
              <X />
            </Button>
          </div>
          <Input
            aria-label="Input label"
            className="h-7 text-xs"
            placeholder="Label"
            value={input.label ?? ""}
            onChange={(e) => {
              setInput(index, { label: e.target.value || undefined })
            }}
          />
          <Input
            aria-label="Input default"
            className="h-7 text-xs"
            placeholder={input.required ? "Required: no default" : "Default (optional)"}
            disabled={input.required}
            value={input.default ?? ""}
            onChange={(e) => {
              setInput(index, { default: e.target.value === "" ? undefined : e.target.value })
            }}
          />
          <div className="flex gap-3">
            <Check
              id={`input-required-${index}`}
              label="Required"
              checked={Boolean(input.required)}
              onChange={(required) => {
                setInput(index, {
                  required: required || undefined,
                  default: required ? undefined : input.default,
                })
              }}
            />
            <Check
              id={`input-sensitive-${index}`}
              label="Sensitive"
              checked={Boolean(input.sensitive)}
              onChange={(sensitive) => {
                setInput(index, { sensitive: sensitive || undefined })
              }}
            />
          </div>
        </div>
      ))}
      <Button
        variant="outline"
        size="sm"
        disabled={inputs.length >= MAX_WORKFLOW_VALUES}
        onClick={() => {
          onInputs([...inputs, { key: nextKey("input", inputs), kind: "text" }])
        }}
      >
        <Plus /> Add input
      </Button>
      <div className="grid gap-1">
        <h3 className="text-xs font-medium text-muted-foreground uppercase">Outputs</h3>
        <p className="text-xs text-muted-foreground">
          Values this workflow returns to a Sub-workflow node; Output nodes set them.
        </p>
      </div>
      {outputs.map((output, index) => (
        <div
          key={index}
          className="grid grid-cols-[minmax(0,1fr)] gap-1.5 rounded-md border p-2"
          role="group"
          aria-label={`Output ${output.key}`}
        >
          <div className="flex items-center gap-1">
            <KeyField
              label="Output key"
              value={output.key}
              onChange={(key) => {
                setOutput(index, { key })
              }}
            />
            <KindSelect
              label="Output type"
              value={output.kind}
              onChange={(kind) => {
                setOutput(index, { kind })
              }}
            />
            <Button
              variant="ghost"
              size="icon"
              className="size-7 shrink-0"
              aria-label={`Remove output ${output.key}`}
              onClick={() => {
                onOutputs(outputs.filter((_, i) => i !== index))
              }}
            >
              <X />
            </Button>
          </div>
          <Input
            aria-label="Output label"
            className="h-7 text-xs"
            placeholder="Label"
            value={output.label ?? ""}
            onChange={(e) => {
              setOutput(index, { label: e.target.value || undefined })
            }}
          />
          <Check
            id={`output-sensitive-${index}`}
            label="Sensitive: not kept in history"
            checked={Boolean(output.sensitive)}
            onChange={(sensitive) => {
              setOutput(index, { sensitive: sensitive || undefined })
            }}
          />
        </div>
      ))}
      <Button
        variant="outline"
        size="sm"
        disabled={outputs.length >= MAX_WORKFLOW_VALUES}
        onClick={() => {
          onOutputs([...outputs, { key: nextKey("output", outputs), kind: "text" }])
        }}
      >
        <Plus /> Add output
      </Button>
    </section>
  )
}
