import { CircleAlert, Plus, TriangleAlert, X } from "lucide-react"
import { useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { GraphMapping, GraphReference, ParameterSpec, VariableDefinition } from "@/lib/types"
import { cn } from "@/lib/utils"

import {
  type OutputOption,
  type Segment,
  VALUE_KIND_LABEL,
  compatibility,
  parseTemplate,
  previewMapping,
  serializeTemplate,
  variableProblems,
} from "./mapping"
import type { VariablePicker } from "./variable-reference"

const key = (r: GraphReference) => `${r.node}.${r.output}`

/** Picks one earlier output; shows each output's type. */
export function OutputSelect({
  id,
  value,
  options,
  onChange,
  className,
}: {
  id?: string
  value: GraphReference | undefined
  options: OutputOption[]
  onChange: (reference: GraphReference) => void
  className?: string
}) {
  const known = value && options.some((o) => key(o.reference) === key(value))
  return (
    <Select
      value={value ? key(value) : undefined}
      onValueChange={(selected) => {
        const option = options.find((o) => key(o.reference) === selected)
        if (option) onChange(option.reference)
      }}
      disabled={options.length === 0 && !value}
    >
      <SelectTrigger id={id} className={cn("w-full min-w-0 [&>span]:truncate", className)}>
        <SelectValue placeholder={options.length ? "Choose an output" : "No earlier outputs"} />
      </SelectTrigger>
      <SelectContent>
        {value && !known && (
          <SelectItem value={key(value)} disabled>
            {key(value)} (missing)
          </SelectItem>
        )}
        {options.map((o) => (
          <SelectItem key={key(o.reference)} value={key(o.reference)}>
            {o.label}
            <span className="text-muted-foreground"> · {VALUE_KIND_LABEL[o.kind]}</span>
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

function Message({ level, children }: { level: "warning" | "error"; children: string }) {
  const Icon = level === "error" ? CircleAlert : TriangleAlert
  return (
    <p
      className={cn(
        "flex gap-1 text-xs",
        level === "error" ? "text-destructive" : "text-amber-700 dark:text-amber-400",
      )}
    >
      <Icon className="mt-0.5 size-3.5 shrink-0" aria-hidden />
      {children}
    </p>
  )
}

/** A typed reference such as `shared.paths.root`, and the picker hook. */
function ReferenceInput({
  label,
  value,
  parameter,
  picker: Picker,
  onChange,
  className,
}: {
  label: string
  value: string
  parameter?: ParameterSpec
  picker?: VariablePicker
  onChange: (reference: string) => void
  className?: string
}) {
  return (
    <div className={cn("flex min-w-0 items-center gap-1", className)}>
      <Input
        aria-label={label}
        className="h-7 min-w-0 flex-1 font-mono text-xs"
        placeholder="shared.paths.root"
        spellCheck={false}
        value={value}
        onChange={(e) => {
          onChange(e.target.value)
        }}
      />
      {Picker && <Picker expected={parameter?.kind} fieldLabel={label} onPick={onChange} />}
    </div>
  )
}

/**
 * A parameter's value from earlier nodes or variables: one output, one
 * variable (shared, system or workflow, keeping its type), or text composed
 * of text, outputs and variables. Shows types, compatibility with the
 * parameter, missing references and a preview. Nothing is evaluated here:
 * variable values are only ever shown by the variable picker.
 */
export function MappingEditor({
  id,
  mapping,
  parameter,
  outputs,
  problem,
  values,
  definitions,
  picker,
  onChange,
}: {
  id: string
  mapping: GraphMapping
  parameter: ParameterSpec
  outputs: OutputOption[]
  /** Why a reference cannot be resolved, or `null`. */
  problem: (reference: GraphReference) => string | null
  /** Outputs of the shown run, for the preview. */
  values?: Record<string, Record<string, string>>
  /** Shared variable definitions, when known, for type checks. */
  definitions?: readonly VariableDefinition[]
  /**
   * The universal variable picker, shown beside every reference field.
   * Without it (or without the agent's `variables` capability) references
   * are typed.
   */
  picker?: VariablePicker
  onChange: (mapping: GraphMapping) => void
}) {
  // The parts being edited. Kept locally so an empty text part (which
  // adds nothing to the stored template) stays while it is being typed;
  // replaced by the stored template when that changes elsewhere (undo).
  const template = mapping.type === "template" ? mapping.template : ""
  const [local, setLocal] = useState<Segment[]>(() => parseTemplate(template))
  const segments = serializeTemplate(local) === template ? local : parseTemplate(template)
  const setSegments = (next: Segment[]) => {
    setLocal(next)
    onChange({ type: "template", template: serializeTemplate(next) })
  }
  const references =
    mapping.type === "output"
      ? [mapping.reference]
      : mapping.type === "template"
        ? segments.flatMap((s) => (s.type === "output" ? [s.reference] : []))
        : []
  const problems = [
    ...references.map(problem).filter((p): p is string => p !== null),
    ...variableProblems(mapping, parameter.kind, definitions),
  ]
  const kind =
    mapping.type === "output"
      ? (outputs.find((o) => key(o.reference) === key(mapping.reference))?.kind ?? "text")
      : "text-template"
  // A variable keeps its type; its fit is checked with the problems above.
  const fit =
    mapping.type === "variable" ? { level: "ok" as const } : compatibility(parameter.kind, kind)
  const first = outputs[0]?.reference

  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-2">
      <Select
        value={mapping.type}
        onValueChange={(type) => {
          if (type === "output" && first) onChange({ type: "output", reference: first })
          if (type === "template") {
            onChange({
              type: "template",
              template:
                mapping.type === "output"
                  ? serializeTemplate([{ type: "output", reference: mapping.reference }])
                  : mapping.type === "variable"
                    ? serializeTemplate([{ type: "variable", reference: mapping.reference }])
                    : "",
            })
          }
          if (type === "variable" && mapping.type !== "variable") {
            onChange({ type: "variable", reference: "" })
          }
        }}
      >
        <SelectTrigger className="h-7 text-xs" aria-label="Mapping">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="output" disabled={outputs.length === 0}>
            One output
          </SelectItem>
          <SelectItem value="variable">One variable</SelectItem>
          <SelectItem value="template">Text with outputs and variables</SelectItem>
        </SelectContent>
      </Select>
      {mapping.type === "variable" ? (
        <ReferenceInput
          label="Variable"
          value={mapping.reference}
          parameter={parameter}
          picker={picker}
          onChange={(reference) => {
            onChange({ ...mapping, reference })
          }}
        />
      ) : mapping.type === "output" ? (
        <>
          <OutputSelect
            id={id}
            value={mapping.reference}
            options={outputs}
            onChange={(reference) => {
              onChange({ ...mapping, reference })
            }}
          />
          <Input
            aria-label="Default value"
            placeholder="Default when the output is missing"
            value={mapping.default ?? ""}
            onChange={(e) => {
              onChange({ ...mapping, default: e.target.value || undefined })
            }}
          />
        </>
      ) : (
        <div className="grid grid-cols-[minmax(0,1fr)] gap-1.5" role="list" aria-label="Parts">
          {segments.length === 0 && (
            <p className="text-xs text-muted-foreground">
              Add text and outputs to compose the value.
            </p>
          )}
          {segments.map((segment, index) => (
            <div key={index} className="flex min-w-0 items-center gap-1" role="listitem">
              {segment.type === "variable" ? (
                <ReferenceInput
                  className="flex-1"
                  label={`Variable ${index + 1}`}
                  value={segment.reference}
                  picker={picker}
                  onChange={(reference) => {
                    setSegments(
                      segments.map((s, i) => (i === index ? { type: "variable", reference } : s)),
                    )
                  }}
                />
              ) : segment.type === "text" ? (
                <Input
                  aria-label={`Text ${index + 1}`}
                  className="h-7 min-w-0 flex-1 text-xs"
                  value={segment.text}
                  onChange={(e) => {
                    setSegments(
                      segments.map((s, i) =>
                        i === index ? { type: "text", text: e.target.value } : s,
                      ),
                    )
                  }}
                />
              ) : (
                <OutputSelect
                  className="h-7 flex-1 text-xs"
                  value={segment.reference}
                  options={outputs}
                  onChange={(reference) => {
                    setSegments(
                      segments.map((s, i) => (i === index ? { type: "output", reference } : s)),
                    )
                  }}
                />
              )}
              <Button
                variant="ghost"
                size="icon"
                className="size-7 shrink-0"
                aria-label="Remove part"
                onClick={() => {
                  setSegments(segments.filter((_, i) => i !== index))
                }}
              >
                <X />
              </Button>
            </div>
          ))}
          <div className="flex gap-1">
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-1.5 text-xs"
              onClick={() => {
                setSegments([...segments, { type: "text", text: "" }])
              }}
            >
              <Plus className="size-3" /> Text
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-1.5 text-xs"
              disabled={!first}
              onClick={() => {
                if (first) setSegments([...segments, { type: "output", reference: first }])
              }}
            >
              <Plus className="size-3" /> Output
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-1.5 text-xs"
              onClick={() => {
                setSegments([...segments, { type: "variable", reference: "" }])
              }}
            >
              <Plus className="size-3" /> Variable
            </Button>
          </div>
        </div>
      )}
      <p className="rounded bg-muted px-2 py-1 text-xs break-words" aria-label="Preview">
        <span className="text-muted-foreground">Preview: </span>
        {previewMapping(mapping, values) || <span className="text-muted-foreground">empty</span>}
      </p>
      {problems.map((p) => (
        <Message key={p} level="error">
          {p}
        </Message>
      ))}
      {fit.level !== "ok" && fit.message && <Message level={fit.level}>{fit.message}</Message>}
    </div>
  )
}
