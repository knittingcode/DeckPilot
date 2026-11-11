import { CircleAlert, Trash2, TriangleAlert } from "lucide-react"
import type { ReactNode } from "react"

import { ParameterField } from "@/components/parameter-field"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { initialValue } from "@/lib/parameters"
import type {
  ErrorPolicy,
  GraphIssue,
  GraphMapping,
  GraphNode,
  GraphPredicate,
  GraphReference,
  ParameterSpec,
  Project,
  WorkflowGraph,
} from "@/lib/types"

import { FLOW_DESCRIPTION, FLOW_TITLE } from "./catalog"
import { type Lookup, actionOf, availableOutputs, formatMs } from "./describe"
import { MAX_DELAY_MS, MAX_LOOP_ITERATIONS, MAX_RETRIES, upstream } from "./graph"

function Field({
  label,
  htmlFor,
  children,
  hint,
}: {
  label: string
  htmlFor?: string
  children: ReactNode
  hint?: ReactNode
}) {
  return (
    <div className="grid gap-1.5">
      <Label htmlFor={htmlFor} className="text-xs">
        {label}
      </Label>
      {children}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  )
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="grid gap-3 border-t px-4 py-3">
      <h3 className="text-xs font-medium text-muted-foreground uppercase">{title}</h3>
      {children}
    </section>
  )
}

export function IssueList({ issues }: { issues: GraphIssue[] }) {
  if (issues.length === 0) return null
  return (
    <ul className="grid gap-1.5" aria-label="Problems">
      {issues.map((issue, index) => (
        <li key={index} className="flex gap-1.5 text-xs">
          {issue.severity === "error" ? (
            <CircleAlert className="mt-0.5 size-3.5 shrink-0 text-destructive" aria-hidden />
          ) : (
            <TriangleAlert
              className="mt-0.5 size-3.5 shrink-0 text-amber-600 dark:text-amber-400"
              aria-hidden
            />
          )}
          <span>
            {issue.node && <span className="font-mono">{issue.node}: </span>}
            {issue.message}
          </span>
        </li>
      ))}
    </ul>
  )
}

const referenceKey = (r: GraphReference) => `${r.node}.${r.output}`

function ReferenceSelect({
  id,
  value,
  options,
  onChange,
}: {
  id: string
  value: GraphReference | undefined
  options: { reference: GraphReference; label: string }[]
  onChange: (reference: GraphReference) => void
}) {
  return (
    <Select
      value={value ? referenceKey(value) : undefined}
      onValueChange={(key) => {
        const option = options.find((o) => referenceKey(o.reference) === key)
        if (option) onChange(option.reference)
      }}
      disabled={options.length === 0}
    >
      <SelectTrigger id={id} className="w-full">
        <SelectValue placeholder={options.length ? "Choose an output" : "No earlier outputs"} />
      </SelectTrigger>
      <SelectContent>
        {options.map((o) => (
          <SelectItem key={referenceKey(o.reference)} value={referenceKey(o.reference)}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

type Change = (change: (node: GraphNode) => GraphNode) => void

export interface InspectorProps {
  graph: WorkflowGraph
  node: GraphNode
  issues: GraphIssue[]
  lookup: Lookup
  projects: Project[]
  /** Other workflows a loop can run. */
  loopTargets: { id: string; name: string }[]
  onChange: Change
  onDelete: () => void
}

/** Configuration of the selected node. */
export function NodeInspector(props: InspectorProps) {
  const { node, issues, onChange, onDelete } = props
  return (
    <div className="grid">
      <div className="grid gap-3 px-4 py-3">
        <div className="flex items-start gap-2">
          <div className="grid flex-1 gap-0.5">
            <h2 className="text-sm font-semibold">{FLOW_TITLE[node.type]}</h2>
            <p className="text-xs text-muted-foreground">{FLOW_DESCRIPTION[node.type]}</p>
          </div>
          {node.type !== "trigger" && (
            <Button
              variant="ghost"
              size="icon"
              className="size-7"
              onClick={onDelete}
              aria-label="Delete node"
            >
              <Trash2 />
            </Button>
          )}
        </div>
        <Field label="Label" htmlFor="node-label">
          <Input
            id="node-label"
            value={node.label ?? ""}
            placeholder="Default title"
            onChange={(e) => {
              const label = e.target.value
              onChange((n) => ({ ...n, label: label || undefined }))
            }}
          />
        </Field>
        <Field label="Node id" hint="Used to refer to this node's outputs.">
          <code className="rounded bg-muted px-2 py-1 font-mono text-xs">{node.id}</code>
        </Field>
        <IssueList issues={issues} />
      </div>
      <KindEditor {...props} />
    </div>
  )
}

function KindEditor(props: InspectorProps) {
  const { node, onChange } = props
  switch (node.type) {
    case "action":
      return <ActionEditor {...props} />
    case "condition":
      return <ConditionEditor {...props} />
    case "loop":
      return (
        <Section title="Loop">
          <Field label="Workflow" htmlFor="loop-workflow">
            <Select
              value={node.workflow}
              onValueChange={(workflow) => {
                onChange((n) => (n.type === "loop" ? { ...n, workflow } : n))
              }}
            >
              <SelectTrigger id="loop-workflow" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {props.loopTargets.map((w) => (
                  <SelectItem key={w.id} value={w.id}>
                    {w.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
          <Field label="Times" htmlFor="loop-times" hint={`1 to ${MAX_LOOP_ITERATIONS}`}>
            <Input
              id="loop-times"
              type="number"
              min={1}
              max={MAX_LOOP_ITERATIONS}
              value={node.times}
              onChange={(e) => {
                const times = Math.trunc(Number(e.target.value))
                onChange((n) => (n.type === "loop" ? { ...n, times } : n))
              }}
            />
          </Field>
        </Section>
      )
    case "delay":
      return (
        <Section title="Delay">
          <Field
            label="Seconds"
            htmlFor="delay-seconds"
            hint={`${formatMs(node.ms)}; at most ${MAX_DELAY_MS / 60000} minutes`}
          >
            <Input
              id="delay-seconds"
              type="number"
              min={0}
              step={0.5}
              max={MAX_DELAY_MS / 1000}
              value={node.ms / 1000}
              onChange={(e) => {
                const ms = Math.max(0, Math.round(Number(e.target.value) * 1000))
                onChange((n) => (n.type === "delay" ? { ...n, ms } : n))
              }}
            />
          </Field>
        </Section>
      )
    default:
      return null
  }
}

type ParameterMode = "action" | "fixed" | "mapped"

function parameterMode(node: GraphNode & { type: "action" }, key: string): ParameterMode {
  if (node.mappings?.[key]) return "mapped"
  if (node.overrides?.[key]) return "fixed"
  return "action"
}

function setParameterMode(
  node: GraphNode & { type: "action" },
  spec: ParameterSpec,
  mode: ParameterMode,
  fallback: GraphMapping,
): GraphNode {
  const without = <T,>(values: Record<string, T> | undefined) =>
    Object.fromEntries(Object.entries(values ?? {}).filter(([key]) => key !== spec.key))
  const overrides = without(node.overrides)
  const mappings = without(node.mappings)
  if (mode === "fixed") overrides[spec.key] = initialValue(spec)
  if (mode === "mapped") mappings[spec.key] = fallback
  return { ...node, overrides, mappings }
}

function ActionEditor({ graph, node, lookup, projects, onChange }: InspectorProps) {
  if (node.type !== "action") return null
  const { action, type } = actionOf(node, lookup)
  const outputs = availableOutputs(graph, node.id, lookup)
  const spec = type?.node
  const policy: ErrorPolicy = node.policy
  const setPolicy = (change: Partial<ErrorPolicy>) => {
    onChange((n) => (n.type === "action" ? { ...n, policy: { ...n.policy, ...change } } : n))
  }
  return (
    <>
      <Section title="Action">
        <Field label="Action" htmlFor="node-action">
          <Select
            value={action ? node.action : undefined}
            onValueChange={(id) => {
              onChange((n) =>
                n.type === "action" ? { ...n, action: id, overrides: {}, mappings: {} } : n,
              )
            }}
          >
            <SelectTrigger id="node-action" className="w-full">
              <SelectValue placeholder="The action no longer exists" />
            </SelectTrigger>
            <SelectContent>
              {lookup.actions.map((a) => (
                <SelectItem key={a.id} value={a.id}>
                  {a.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        {type?.description && <p className="text-xs text-muted-foreground">{type.description}</p>}
      </Section>
      {type && type.parameters.length > 0 && (
        <Section title="Parameters">
          {type.parameters.map((parameter) => {
            const mode = parameterMode(node, parameter.key)
            const mappable = parameter.kind.type !== "resource"
            const first = outputs[0]?.reference
            const fallback: GraphMapping = first
              ? { type: "output", reference: first }
              : { type: "template", template: "" }
            const mapping = node.mappings?.[parameter.key]
            return (
              <div key={parameter.key} className="grid gap-2 rounded-md border p-2.5">
                <div className="flex items-center gap-2">
                  <span className="flex-1 text-xs font-medium">{parameter.label}</span>
                  <Select
                    value={mode}
                    onValueChange={(value) => {
                      onChange((n) =>
                        n.type === "action"
                          ? setParameterMode(n, parameter, value as ParameterMode, fallback)
                          : n,
                      )
                    }}
                  >
                    <SelectTrigger
                      className="h-7 w-36 text-xs"
                      aria-label={`${parameter.label} source`}
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="action">Action’s value</SelectItem>
                      <SelectItem value="fixed">Value for this node</SelectItem>
                      {mappable && <SelectItem value="mapped">From earlier nodes</SelectItem>}
                    </SelectContent>
                  </Select>
                </div>
                {mode === "fixed" && (
                  <ParameterField
                    spec={{ ...parameter, label: "Value" }}
                    value={node.overrides?.[parameter.key] ?? initialValue(parameter)}
                    projects={projects}
                    onChange={(value) => {
                      onChange((n) =>
                        n.type === "action"
                          ? { ...n, overrides: { ...n.overrides, [parameter.key]: value } }
                          : n,
                      )
                    }}
                  />
                )}
                {mode === "mapped" && mapping && (
                  <MappingEditor
                    id={`mapping-${parameter.key}`}
                    mapping={mapping}
                    outputs={outputs}
                    onChange={(next) => {
                      onChange((n) =>
                        n.type === "action"
                          ? { ...n, mappings: { ...n.mappings, [parameter.key]: next } }
                          : n,
                      )
                    }}
                  />
                )}
              </div>
            )
          })}
        </Section>
      )}
      <Section title="On failure">
        <Field
          label="When the action fails"
          htmlFor="node-on-error"
          hint="A connected “On error” output always handles the failure."
        >
          <Select
            value={policy.onError}
            onValueChange={(value) => {
              setPolicy({ onError: value as ErrorPolicy["onError"] })
            }}
          >
            <SelectTrigger id="node-on-error" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="stop">Stop the workflow</SelectItem>
              <SelectItem value="continue">Continue with Next</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <div className="grid grid-cols-2 gap-2">
          <Field
            label="Retries"
            htmlFor="node-retries"
            hint={spec?.retrySafe ? `Up to ${MAX_RETRIES}` : "Not safe to repeat"}
          >
            <Input
              id="node-retries"
              type="number"
              min={0}
              max={MAX_RETRIES}
              disabled={!spec?.retrySafe}
              value={policy.retry.attempts}
              onChange={(e) => {
                setPolicy({
                  retry: { ...policy.retry, attempts: Math.trunc(Number(e.target.value)) },
                })
              }}
            />
          </Field>
          <Field label="Wait between (ms)" htmlFor="node-backoff" hint="Doubles each time">
            <Input
              id="node-backoff"
              type="number"
              min={0}
              step={100}
              disabled={!spec?.retrySafe || policy.retry.attempts === 0}
              value={policy.retry.backoffMs}
              onChange={(e) => {
                setPolicy({
                  retry: {
                    ...policy.retry,
                    backoffMs: Math.max(0, Math.trunc(Number(e.target.value))),
                  },
                })
              }}
            />
          </Field>
        </div>
        <Field
          label="Timeout (seconds)"
          htmlFor="node-timeout"
          hint={`Default ${formatMs(spec?.timeoutMs ?? 30000)}`}
        >
          <Input
            id="node-timeout"
            type="number"
            min={1}
            placeholder={String((spec?.timeoutMs ?? 30000) / 1000)}
            value={policy.timeoutMs === undefined ? "" : policy.timeoutMs / 1000}
            onChange={(e) => {
              const text = e.target.value
              setPolicy({
                timeoutMs: text === "" ? undefined : Math.max(1, Math.round(Number(text) * 1000)),
              })
            }}
          />
        </Field>
      </Section>
      {spec && spec.outputs.length > 0 && (
        <Section title="Outputs">
          <ul className="grid gap-1 text-xs">
            {spec.outputs.map((output) => (
              <li key={output.key} className="flex gap-2">
                <code className="font-mono">{output.key}</code>
                <span className="text-muted-foreground">{output.label}</span>
              </li>
            ))}
          </ul>
        </Section>
      )}
    </>
  )
}

function MappingEditor({
  id,
  mapping,
  outputs,
  onChange,
}: {
  id: string
  mapping: GraphMapping
  outputs: { reference: GraphReference; label: string }[]
  onChange: (mapping: GraphMapping) => void
}) {
  return (
    <div className="grid gap-2">
      <Select
        value={mapping.type}
        onValueChange={(type) => {
          const first = outputs[0]?.reference
          if (type === "output" && first) onChange({ type: "output", reference: first })
          if (type === "template") onChange({ type: "template", template: "" })
        }}
      >
        <SelectTrigger className="h-7 text-xs" aria-label="Mapping">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="output" disabled={outputs.length === 0}>
            One output
          </SelectItem>
          <SelectItem value="template">Text with outputs</SelectItem>
        </SelectContent>
      </Select>
      {mapping.type === "output" ? (
        <>
          <ReferenceSelect
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
        <>
          <Textarea
            id={id}
            rows={2}
            className="font-mono text-xs"
            placeholder="{{ steps.node.output.key }}"
            value={mapping.template}
            onChange={(e) => {
              onChange({ type: "template", template: e.target.value })
            }}
          />
          {outputs.length > 0 && (
            <Select
              value=""
              onValueChange={(key) => {
                const option = outputs.find((o) => referenceKey(o.reference) === key)
                if (option) {
                  const token = `{{ steps.${option.reference.node}.output.${option.reference.output} }}`
                  onChange({ type: "template", template: `${mapping.template}${token}` })
                }
              }}
            >
              <SelectTrigger className="h-7 text-xs" aria-label="Insert an output">
                <SelectValue placeholder="Insert an output…" />
              </SelectTrigger>
              <SelectContent>
                {outputs.map((o) => (
                  <SelectItem key={referenceKey(o.reference)} value={referenceKey(o.reference)}>
                    {o.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
        </>
      )}
    </div>
  )
}

const PREDICATES: { value: GraphPredicate["type"]; label: string }[] = [
  { value: "succeeded", label: "A node succeeded" },
  { value: "equals", label: "An output equals" },
  { value: "notEmpty", label: "An output is set" },
  { value: "activeProject", label: "The active project is" },
]

function ConditionEditor({ graph, node, lookup, projects, onChange }: InspectorProps) {
  if (node.type !== "condition") return null
  const predicate = node.predicate
  const outputs = availableOutputs(graph, node.id, lookup)
  const earlier = upstream(graph, node.id)
  const set = (next: GraphPredicate) => {
    onChange((n) => (n.type === "condition" ? { ...n, predicate: next } : n))
  }
  const defaults = (type: GraphPredicate["type"]): GraphPredicate | null => {
    const reference = outputs[0]?.reference
    switch (type) {
      case "succeeded":
        return { type, node: earlier.at(-1) ?? "start" }
      case "equals":
        return reference ? { type, reference, value: "" } : null
      case "notEmpty":
        return reference ? { type, reference } : null
      case "activeProject":
        return { type, key: projects[0]?.key ?? "" }
    }
  }
  return (
    <Section title="Condition">
      <Field label="Check" htmlFor="condition-type">
        <Select
          value={predicate.type}
          onValueChange={(type) => {
            const next = defaults(type as GraphPredicate["type"])
            if (next) set(next)
          }}
        >
          <SelectTrigger id="condition-type" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {PREDICATES.map((p) => (
              <SelectItem key={p.value} value={p.value} disabled={defaults(p.value) === null}>
                {p.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>
      {predicate.type === "succeeded" && (
        <Field label="Node" htmlFor="condition-node">
          <Select
            value={predicate.node}
            onValueChange={(id) => {
              set({ type: "succeeded", node: id })
            }}
          >
            <SelectTrigger id="condition-node" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {[...new Set([...earlier, predicate.node])].map((id) => (
                <SelectItem key={id} value={id}>
                  {graph.nodes.find((n) => n.id === id)?.label || id}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
      )}
      {(predicate.type === "equals" || predicate.type === "notEmpty") && (
        <Field label="Output" htmlFor="condition-reference">
          <ReferenceSelect
            id="condition-reference"
            value={predicate.reference}
            options={outputs}
            onChange={(reference) => {
              set({ ...predicate, reference })
            }}
          />
        </Field>
      )}
      {predicate.type === "equals" && (
        <Field label="Value" htmlFor="condition-value">
          <Input
            id="condition-value"
            value={predicate.value}
            onChange={(e) => {
              set({ ...predicate, value: e.target.value })
            }}
          />
        </Field>
      )}
      {predicate.type === "activeProject" && (
        <Field label="Project" htmlFor="condition-project">
          <Select
            value={predicate.key || undefined}
            onValueChange={(key) => {
              set({ type: "activeProject", key })
            }}
          >
            <SelectTrigger id="condition-project" className="w-full">
              <SelectValue placeholder="Choose a project" />
            </SelectTrigger>
            <SelectContent>
              {projects.map((p) => (
                <SelectItem key={p.id} value={p.key}>
                  {p.key} · {p.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
      )}
      <p className="text-xs text-muted-foreground">
        Yes and No continue with different nodes; connect them on the canvas.
      </p>
    </Section>
  )
}
