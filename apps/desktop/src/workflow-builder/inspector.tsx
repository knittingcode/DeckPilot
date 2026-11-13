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
import { initialValue } from "@/lib/parameters"
import type {
  ErrorPolicy,
  GraphIssue,
  GraphMapping,
  GraphNode,
  GraphPredicate,
  ParameterSpec,
  Project,
  ValueKind,
  WorkflowGraph,
} from "@/lib/types"

import { FLOW_DESCRIPTION, FLOW_TITLE } from "./catalog"
import type { NodeRun } from "./node-view"
import { NodeRunReport } from "./run-panel"
import { type Lookup, actionOf, formatMs } from "./describe"
import { MappingEditor, OutputSelect } from "./mapping-editor"
import { VARIABLE_PICKER } from "./variable-reference"
import {
  type OutputOption,
  VALUE_KIND_LABEL,
  outputOptions,
  parameterKindOf,
  referenceProblem,
} from "./mapping"
import { NodeDocumentation } from "./node-docs"
import {
  DEFAULT_CALL_TIMEOUT_MS,
  MAX_DELAY_MS,
  MAX_LOOP_ITERATIONS,
  MAX_RETRIES,
  upstream,
} from "./graph"

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
    <section className="grid grid-cols-[minmax(0,1fr)] gap-3 border-t px-4 py-3">
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

type Change = (change: (node: GraphNode) => GraphNode) => void

export interface InspectorProps {
  graph: WorkflowGraph
  node: GraphNode
  issues: GraphIssue[]
  lookup: Lookup
  projects: Project[]
  /** Other workflows a loop or sub-workflow can run. */
  loopTargets: { id: string; name: string }[]
  onChange: Change
  onDelete: () => void
  /** The node in the run shown on the canvas. */
  run?: NodeRun
  /** Outputs of the shown run per node, for mapping previews. */
  values?: Record<string, Record<string, string>>
}

/** Configuration of the selected node. */
export function NodeInspector(props: InspectorProps) {
  const { node, issues, onChange, onDelete } = props
  return (
    <div className="grid grid-cols-[minmax(0,1fr)]">
      <div className="grid grid-cols-[minmax(0,1fr)] gap-3 px-4 py-3">
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
      {props.run && <NodeRunReport run={props.run} />}
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
    case "call":
      return <CallEditor {...props} />
    case "output":
      return <OutputEditor {...props} />
    case "setVariable":
      return <SetVariableEditor {...props} />
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

function ActionEditor({ graph, node, lookup, projects, values, onChange }: InspectorProps) {
  if (node.type !== "action") return null
  const { action, type } = actionOf(node, lookup)
  const outputs = outputOptions(graph, node.id, lookup)
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
            // A resource comes only from a variable that holds one.
            const resource = parameter.kind.type === "resource"
            const first = outputs[0]?.reference
            const fallback: GraphMapping = resource
              ? { type: "variable", reference: "" }
              : first
                ? { type: "output", reference: first }
                : { type: "template", template: "" }
            const mapping = node.mappings?.[parameter.key]
            return (
              <div
                key={parameter.key}
                className="grid grid-cols-[minmax(0,1fr)] gap-2 rounded-md border p-2.5"
              >
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
                      <SelectItem value="mapped">From earlier nodes or variables</SelectItem>
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
                    picker={VARIABLE_PICKER}
                    mapping={mapping}
                    parameter={parameter}
                    outputs={outputs}
                    values={values}
                    problem={(reference) => referenceProblem(graph, node.id, reference, lookup)}
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
          hint={`Default ${formatMs(spec?.timeoutMs ?? 30000)}. ${
            spec?.cancellable
              ? "Stops at once when the run is cancelled or the time is up."
              : "Cannot be interrupted: it finishes before a cancellation or timeout takes effect, and a late result is discarded."
          }`}
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
                <span className="flex-1 text-muted-foreground">{output.label}</span>
                <span className="text-muted-foreground">{VALUE_KIND_LABEL[output.kind]}</span>
              </li>
            ))}
          </ul>
        </Section>
      )}
      {type && <NodeDocumentation type={type} lookup={lookup} />}
    </>
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
  const outputs = outputOptions(graph, node.id, lookup)
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
          <OutputSelect
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

/** One value from earlier nodes for an input or output, or none. */
function ValueRow({
  id,
  label,
  kind,
  required,
  sensitive,
  fallbackText,
  mapping,
  outputs,
  graph,
  node,
  lookup,
  values,
  onChange,
}: {
  id: string
  label: string
  kind: ValueKind
  required?: boolean
  sensitive?: boolean
  /** What happens without a value. */
  fallbackText: string
  mapping: GraphMapping | undefined
  outputs: OutputOption[]
  graph: WorkflowGraph
  node: GraphNode
  lookup: Lookup
  values?: Record<string, Record<string, string>>
  onChange: (mapping: GraphMapping | undefined) => void
}) {
  const first = outputs[0]?.reference
  const fallback: GraphMapping = first
    ? { type: "output", reference: first }
    : { type: "template", template: "" }
  const parameter: ParameterSpec = {
    key: id,
    label,
    kind: parameterKindOf(kind),
    required: Boolean(required),
  }
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-2 rounded-md border p-2.5">
      <div className="flex items-center gap-2">
        <span className="flex-1 text-xs font-medium">
          {label}
          <span className="font-normal text-muted-foreground">
            {" "}
            · {VALUE_KIND_LABEL[kind]}
            {required ? " · required" : ""}
            {sensitive ? " · sensitive" : ""}
          </span>
        </span>
        <Select
          value={mapping ? "mapped" : "unset"}
          onValueChange={(mode) => {
            onChange(mode === "mapped" ? fallback : undefined)
          }}
        >
          <SelectTrigger className="h-7 w-36 text-xs" aria-label={`${label} source`}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="unset">No value</SelectItem>
            <SelectItem value="mapped">From earlier nodes or variables</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {mapping ? (
        <MappingEditor
          id={`value-${id}`}
          picker={VARIABLE_PICKER}
          mapping={mapping}
          parameter={parameter}
          outputs={outputs}
          values={values}
          problem={(reference) => referenceProblem(graph, node.id, reference, lookup)}
          onChange={onChange}
        />
      ) : (
        <p className={required ? "text-xs text-destructive" : "text-xs text-muted-foreground"}>
          {required ? "Needs a value." : fallbackText}
        </p>
      )}
    </div>
  )
}

function withValue(
  values: Record<string, GraphMapping> | undefined,
  key: string,
  mapping: GraphMapping | undefined,
): Record<string, GraphMapping> {
  const next = Object.fromEntries(Object.entries(values ?? {}).filter(([k]) => k !== key))
  if (mapping) next[key] = mapping
  return next
}

/** A sub-workflow: the workflow it runs, its inputs and failure handling. */
function CallEditor({ graph, node, lookup, values, loopTargets, onChange }: InspectorProps) {
  if (node.type !== "call") return null
  const callee = lookup.workflows.find((w) => w.id === node.workflow)
  const outputs = outputOptions(graph, node.id, lookup)
  const policy = node.policy
  const setPolicy = (change: Partial<ErrorPolicy>) => {
    onChange((n) => (n.type === "call" ? { ...n, policy: { ...n.policy, ...change } } : n))
  }
  const declared = callee?.inputs ?? []
  const unknown = Object.keys(node.inputs ?? {}).filter(
    (key) => !declared.some((input) => input.key === key),
  )
  return (
    <>
      <Section title="Sub-workflow">
        <Field
          label="Workflow"
          htmlFor="call-workflow"
          hint="Runs once, with this run's project context; this run waits for it."
        >
          <Select
            value={callee ? node.workflow : undefined}
            onValueChange={(workflow) => {
              onChange((n) => (n.type === "call" ? { ...n, workflow, inputs: {} } : n))
            }}
          >
            <SelectTrigger id="call-workflow" className="w-full">
              <SelectValue placeholder="The workflow no longer exists" />
            </SelectTrigger>
            <SelectContent>
              {loopTargets.map((w) => (
                <SelectItem key={w.id} value={w.id}>
                  {w.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        {callee?.description && (
          <p className="text-xs text-muted-foreground">{callee.description}</p>
        )}
      </Section>
      {callee && (
        <Section title="Inputs">
          {declared.length === 0 && (
            <p className="text-xs text-muted-foreground">
              {callee.name} declares no inputs. Inputs are declared in its workflow panel.
            </p>
          )}
          {declared.map((input) => (
            <ValueRow
              key={input.key}
              id={input.key}
              label={input.label || input.key}
              kind={input.kind}
              required={input.required}
              sensitive={input.sensitive}
              fallbackText={
                input.default !== undefined
                  ? `Uses the default “${input.default}”.`
                  : "Not set; the workflow gets no value."
              }
              mapping={node.inputs?.[input.key]}
              outputs={outputs}
              graph={graph}
              node={node}
              lookup={lookup}
              values={values}
              onChange={(mapping) => {
                onChange((n) =>
                  n.type === "call" ? { ...n, inputs: withValue(n.inputs, input.key, mapping) } : n,
                )
              }}
            />
          ))}
          {unknown.map((key) => (
            <div key={key} className="flex items-center gap-2 text-xs text-destructive">
              <span className="flex-1">
                <code className="font-mono">{key}</code> is no longer an input of {callee.name}.
              </span>
              <Button
                variant="ghost"
                size="sm"
                className="h-6 px-1.5 text-xs"
                onClick={() => {
                  onChange((n) =>
                    n.type === "call" ? { ...n, inputs: withValue(n.inputs, key, undefined) } : n,
                  )
                }}
              >
                Remove
              </Button>
            </div>
          ))}
        </Section>
      )}
      <Section title="On failure">
        <Field
          label="When the sub-workflow fails"
          htmlFor="call-on-error"
          hint="A connected “On error” output always handles the failure. Sub-workflows are not retried."
        >
          <Select
            value={policy.onError}
            onValueChange={(value) => {
              setPolicy({ onError: value as ErrorPolicy["onError"] })
            }}
          >
            <SelectTrigger id="call-on-error" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="stop">Stop the workflow</SelectItem>
              <SelectItem value="continue">Continue with Next</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <Field
          label="Timeout (seconds)"
          htmlFor="call-timeout"
          hint={`Default ${formatMs(DEFAULT_CALL_TIMEOUT_MS)}. Cancelling the run or reaching the timeout stops the sub-workflow too.`}
        >
          <Input
            id="call-timeout"
            type="number"
            min={1}
            placeholder={String(DEFAULT_CALL_TIMEOUT_MS / 1000)}
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
      {callee && (
        <Section title="Outputs">
          {(callee.outputs ?? []).length === 0 ? (
            <p className="text-xs text-muted-foreground">{callee.name} returns no outputs.</p>
          ) : (
            <ul className="grid gap-1 text-xs" aria-label="Sub-workflow outputs">
              {(callee.outputs ?? []).map((output) => (
                <li key={output.key} className="flex gap-2">
                  <code className="font-mono">{output.key}</code>
                  <span className="flex-1 text-muted-foreground">
                    {output.label || output.key}
                    {output.sensitive ? " · not kept in history" : ""}
                  </span>
                  <span className="text-muted-foreground">{VALUE_KIND_LABEL[output.kind]}</span>
                </li>
              ))}
            </ul>
          )}
        </Section>
      )}
    </>
  )
}

/** An Output node: values of the open workflow's declared outputs. */
function OutputEditor({ graph, node, lookup, values, onChange }: InspectorProps) {
  if (node.type !== "output") return null
  const declared = lookup.outputs ?? []
  const outputs = outputOptions(graph, node.id, lookup)
  return (
    <Section title="Outputs">
      {declared.length === 0 && (
        <p className="text-xs text-muted-foreground">
          This workflow declares no outputs. Declare them in the workflow panel (click the canvas
          background).
        </p>
      )}
      {declared.map((output) => (
        <ValueRow
          key={output.key}
          id={output.key}
          label={output.label || output.key}
          kind={output.kind}
          sensitive={output.sensitive}
          fallbackText="Not set by this node."
          mapping={node.values?.[output.key]}
          outputs={outputs}
          graph={graph}
          node={node}
          lookup={lookup}
          values={values}
          onChange={(mapping) => {
            onChange((n) =>
              n.type === "output" ? { ...n, values: withValue(n.values, output.key, mapping) } : n,
            )
          }}
        />
      ))}
      <p className="text-xs text-muted-foreground">
        The node that calls this workflow gets these values. When several Output nodes run, the one
        that finishes later wins.
      </p>
    </Section>
  )
}

/** A Set variable node: new values of the open workflow's variables. */
function SetVariableEditor({ graph, node, lookup, values, onChange }: InspectorProps) {
  if (node.type !== "setVariable") return null
  const declared = lookup.variables ?? []
  const outputs = outputOptions(graph, node.id, lookup)
  return (
    <Section title="Variables">
      {declared.length === 0 && (
        <p className="text-xs text-muted-foreground">
          This workflow declares no variables. Declare them in the workflow panel (click the canvas
          background).
        </p>
      )}
      {declared.map((variable) => (
        <ValueRow
          key={variable.key}
          id={`variable-${variable.key}`}
          label={variable.label || variable.key}
          kind={variable.kind}
          sensitive={variable.sensitive}
          fallbackText="Not changed by this node."
          mapping={node.values?.[variable.key]}
          outputs={outputs}
          graph={graph}
          node={node}
          lookup={lookup}
          values={values}
          onChange={(mapping) => {
            onChange((n) =>
              n.type === "setVariable"
                ? { ...n, values: withValue(n.values, variable.key, mapping) }
                : n,
            )
          }}
        />
      ))}
      <p className="text-xs text-muted-foreground">
        Nodes after this one read the new values as “Variable · …”. Variables last for one run; a
        sub-workflow gets them only through its inputs.
      </p>
    </Section>
  )
}
