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
import { Trans, type TranslationKey, useTranslation } from "@/i18n"
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
  const { t } = useTranslation()
  if (issues.length === 0) return null
  return (
    <ul className="grid gap-1.5" aria-label={t("workflows:inspector.problems")}>
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
  const { t } = useTranslation()
  const { node, issues, onChange, onDelete } = props
  return (
    <div className="grid grid-cols-[minmax(0,1fr)]">
      <div className="grid grid-cols-[minmax(0,1fr)] gap-3 px-4 py-3">
        <div className="flex items-start gap-2">
          <div className="grid flex-1 gap-0.5">
            <h2 className="text-sm font-semibold">{t(FLOW_TITLE[node.type])}</h2>
            <p className="text-xs text-muted-foreground">{t(FLOW_DESCRIPTION[node.type])}</p>
          </div>
          {node.type !== "trigger" && (
            <Button
              variant="ghost"
              size="icon"
              className="size-7"
              onClick={onDelete}
              aria-label={t("workflows:inspector.deleteNode")}
            >
              <Trash2 />
            </Button>
          )}
        </div>
        <Field label={t("workflows:inspector.label")} htmlFor="node-label">
          <Input
            id="node-label"
            value={node.label ?? ""}
            placeholder={t("workflows:inspector.labelPlaceholder")}
            onChange={(e) => {
              const label = e.target.value
              onChange((n) => ({ ...n, label: label || undefined }))
            }}
          />
        </Field>
        <Field label={t("workflows:inspector.nodeId")} hint={t("workflows:inspector.nodeIdHint")}>
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
  const { t } = useTranslation()
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
        <Section title={t("workflows:node.title.loop")}>
          <Field label={t("workflows:inspector.workflow")} htmlFor="loop-workflow">
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
          <Field
            label={t("workflows:inspector.loop.times")}
            htmlFor="loop-times"
            hint={t("workflows:inspector.loop.timesHint", { max: MAX_LOOP_ITERATIONS })}
          >
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
        <Section title={t("workflows:node.title.delay")}>
          <Field
            label={t("workflows:inspector.delay.seconds")}
            htmlFor="delay-seconds"
            hint={t("workflows:inspector.delay.hint", {
              duration: formatMs(node.ms, t),
              count: MAX_DELAY_MS / 60000,
            })}
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
  const { t } = useTranslation()
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
      <Section title={t("workflows:node.title.action")}>
        <Field label={t("workflows:node.title.action")} htmlFor="node-action">
          <Select
            value={action ? node.action : undefined}
            onValueChange={(id) => {
              onChange((n) =>
                n.type === "action" ? { ...n, action: id, overrides: {}, mappings: {} } : n,
              )
            }}
          >
            <SelectTrigger id="node-action" className="w-full">
              <SelectValue placeholder={t("workflows:node.actionGone")} />
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
        <Section title={t("workflows:inspector.parameters")}>
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
                      aria-label={t("workflows:inspector.source", { label: parameter.label })}
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="action">{t("workflows:inspector.mode.action")}</SelectItem>
                      <SelectItem value="fixed">{t("workflows:inspector.mode.fixed")}</SelectItem>
                      <SelectItem value="mapped">{t("workflows:inspector.mode.mapped")}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                {mode === "fixed" && (
                  <ParameterField
                    spec={{ ...parameter, label: t("workflows:inspector.value") }}
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
      <Section title={t("workflows:inspector.failure.title")}>
        <Field
          label={t("workflows:inspector.failure.whenAction")}
          htmlFor="node-on-error"
          hint={t("workflows:inspector.failure.actionHint")}
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
              <SelectItem value="stop">{t("workflows:inspector.failure.stop")}</SelectItem>
              <SelectItem value="continue">{t("workflows:inspector.failure.continue")}</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <div className="grid grid-cols-2 gap-2">
          <Field
            label={t("workflows:inspector.failure.retries")}
            htmlFor="node-retries"
            hint={
              spec?.retrySafe
                ? t("workflows:inspector.failure.retriesHint", { max: MAX_RETRIES })
                : t("workflows:inspector.failure.notRetrySafe")
            }
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
          <Field
            label={t("workflows:inspector.failure.backoff")}
            htmlFor="node-backoff"
            hint={t("workflows:inspector.failure.backoffHint")}
          >
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
          label={t("workflows:inspector.failure.timeout")}
          htmlFor="node-timeout"
          hint={t(
            spec?.cancellable
              ? "workflows:inspector.failure.timeoutCancellable"
              : "workflows:inspector.failure.timeoutNotCancellable",
            { duration: formatMs(spec?.timeoutMs ?? 30000, t) },
          )}
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
        <Section title={t("workflows:node.title.output")}>
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

const PREDICATES: { value: GraphPredicate["type"]; label: TranslationKey }[] = [
  { value: "succeeded", label: "workflows:inspector.condition.succeeded" },
  { value: "equals", label: "workflows:inspector.condition.equals" },
  { value: "notEmpty", label: "workflows:inspector.condition.notEmpty" },
  { value: "activeProject", label: "workflows:inspector.condition.activeProject" },
]

function ConditionEditor({ graph, node, lookup, projects, onChange }: InspectorProps) {
  const { t } = useTranslation()
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
    <Section title={t("workflows:node.title.condition")}>
      <Field label={t("workflows:inspector.condition.check")} htmlFor="condition-type">
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
                {t(p.label)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>
      {predicate.type === "succeeded" && (
        <Field label={t("workflows:inspector.condition.node")} htmlFor="condition-node">
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
        <Field label={t("workflows:inspector.condition.output")} htmlFor="condition-reference">
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
        <Field label={t("workflows:inspector.value")} htmlFor="condition-value">
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
        <Field label={t("workflows:inspector.condition.project")} htmlFor="condition-project">
          <Select
            value={predicate.key || undefined}
            onValueChange={(key) => {
              set({ type: "activeProject", key })
            }}
          >
            <SelectTrigger id="condition-project" className="w-full">
              <SelectValue placeholder={t("workflows:inspector.condition.chooseProject")} />
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
      <p className="text-xs text-muted-foreground">{t("workflows:inspector.condition.hint")}</p>
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
  const { t } = useTranslation()
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
            {required ? ` · ${t("workflows:inspector.valueRow.required")}` : ""}
            {sensitive ? ` · ${t("workflows:inspector.valueRow.sensitive")}` : ""}
          </span>
        </span>
        <Select
          value={mapping ? "mapped" : "unset"}
          onValueChange={(mode) => {
            onChange(mode === "mapped" ? fallback : undefined)
          }}
        >
          <SelectTrigger
            className="h-7 w-36 text-xs"
            aria-label={t("workflows:inspector.source", { label })}
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="unset">{t("workflows:inspector.valueRow.none")}</SelectItem>
            <SelectItem value="mapped">{t("workflows:inspector.mode.mapped")}</SelectItem>
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
          {required ? t("workflows:inspector.valueRow.needsValue") : fallbackText}
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
  const { t } = useTranslation()
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
      <Section title={t("workflows:node.title.call")}>
        <Field
          label={t("workflows:inspector.workflow")}
          htmlFor="call-workflow"
          hint={t("workflows:inspector.call.hint")}
        >
          <Select
            value={callee ? node.workflow : undefined}
            onValueChange={(workflow) => {
              onChange((n) => (n.type === "call" ? { ...n, workflow, inputs: {} } : n))
            }}
          >
            <SelectTrigger id="call-workflow" className="w-full">
              <SelectValue placeholder={t("workflows:inspector.call.gone")} />
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
        <Section title={t("workflows:inspector.call.inputs")}>
          {declared.length === 0 && (
            <p className="text-xs text-muted-foreground">
              {t("workflows:inspector.call.noInputs", { name: callee.name })}
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
                  ? t("workflows:inspector.call.usesDefault", { value: input.default })
                  : t("workflows:inspector.call.notSet")
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
                <Trans
                  i18nKey="workflows:inspector.call.unknownInput"
                  values={{ key, workflow: callee.name }}
                  components={{ code: <code className="font-mono" /> }}
                />
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
                {t("workflows:inspector.call.remove")}
              </Button>
            </div>
          ))}
        </Section>
      )}
      <Section title={t("workflows:inspector.failure.title")}>
        <Field
          label={t("workflows:inspector.failure.whenCall")}
          htmlFor="call-on-error"
          hint={t("workflows:inspector.failure.callHint")}
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
              <SelectItem value="stop">{t("workflows:inspector.failure.stop")}</SelectItem>
              <SelectItem value="continue">{t("workflows:inspector.failure.continue")}</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <Field
          label={t("workflows:inspector.failure.timeout")}
          htmlFor="call-timeout"
          hint={t("workflows:inspector.failure.callTimeoutHint", {
            duration: formatMs(DEFAULT_CALL_TIMEOUT_MS, t),
          })}
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
        <Section title={t("workflows:node.title.output")}>
          {(callee.outputs ?? []).length === 0 ? (
            <p className="text-xs text-muted-foreground">
              {t("workflows:inspector.call.noOutputs", { name: callee.name })}
            </p>
          ) : (
            <ul className="grid gap-1 text-xs" aria-label={t("workflows:inspector.call.outputs")}>
              {(callee.outputs ?? []).map((output) => (
                <li key={output.key} className="flex gap-2">
                  <code className="font-mono">{output.key}</code>
                  <span className="flex-1 text-muted-foreground">
                    {output.label || output.key}
                    {output.sensitive ? ` · ${t("workflows:inspector.docs.notKept")}` : ""}
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
  const { t } = useTranslation()
  if (node.type !== "output") return null
  const declared = lookup.outputs ?? []
  const outputs = outputOptions(graph, node.id, lookup)
  return (
    <Section title={t("workflows:node.title.output")}>
      {declared.length === 0 && (
        <p className="text-xs text-muted-foreground">{t("workflows:inspector.output.none")}</p>
      )}
      {declared.map((output) => (
        <ValueRow
          key={output.key}
          id={output.key}
          label={output.label || output.key}
          kind={output.kind}
          sensitive={output.sensitive}
          fallbackText={t("workflows:inspector.output.notSet")}
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
      <p className="text-xs text-muted-foreground">{t("workflows:inspector.output.hint")}</p>
    </Section>
  )
}

/** A Set variable node: new values of the open workflow's variables. */
function SetVariableEditor({ graph, node, lookup, values, onChange }: InspectorProps) {
  const { t } = useTranslation()
  if (node.type !== "setVariable") return null
  const declared = lookup.variables ?? []
  const outputs = outputOptions(graph, node.id, lookup)
  return (
    <Section title={t("workflows:inspector.variables.title")}>
      {declared.length === 0 && (
        <p className="text-xs text-muted-foreground">{t("workflows:inspector.variables.none")}</p>
      )}
      {declared.map((variable) => (
        <ValueRow
          key={variable.key}
          id={`variable-${variable.key}`}
          label={variable.label || variable.key}
          kind={variable.kind}
          sensitive={variable.sensitive}
          fallbackText={t("workflows:inspector.variables.notChanged")}
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
      <p className="text-xs text-muted-foreground">{t("workflows:inspector.variables.hint")}</p>
    </Section>
  )
}
