import { Handle, type Node, type NodeProps, Position } from "@xyflow/react"
import {
  CircleAlert,
  CircleCheck,
  CircleSlash,
  GitBranch,
  Loader2,
  Merge,
  OctagonX,
  Play,
  Repeat,
  SkipForward,
  Split,
  Timer,
  TriangleAlert,
} from "lucide-react"
import { createElement } from "react"

import { ActionIcon } from "@/components/action-icon"
import type { GraphIssue, GraphPort, NodeProgress, NodeReport } from "@/lib/types"
import { cn } from "@/lib/utils"

import { type NodeType, PORT_LABEL, hasInput, outputPorts } from "./graph"
import { formatMs } from "./describe"

/** How a node did in the run shown on the canvas. */
export type NodeRun =
  | { state: "running"; attempt?: number; progress?: NodeProgress }
  | { state: "done"; report: NodeReport }

export interface NodeViewData extends Record<string, unknown> {
  type: NodeType
  title: string
  detail: string
  icon: string | null
  issues: GraphIssue[]
  run?: NodeRun
}

export type FlowNode = Node<NodeViewData, "workflow">

const FLOW_ICONS = {
  trigger: Play,
  condition: GitBranch,
  parallel: Split,
  join: Merge,
  loop: Repeat,
  delay: Timer,
  stop: OctagonX,
} as const

const PORT_CLASS: Record<GraphPort, string> = {
  out: "!bg-primary",
  error: "!bg-destructive",
  true: "!bg-emerald-600",
  false: "!bg-amber-500",
}

function NodeIcon({ data }: { data: NodeViewData }) {
  if (data.type === "action") return <ActionIcon name={data.icon} className="size-4" />
  return createElement(FLOW_ICONS[data.type], { className: "size-4", "aria-hidden": true })
}

function RunBadge({ run }: { run: NodeRun }) {
  if (run.state === "running") {
    return (
      <span className="flex items-center gap-1 text-xs text-primary">
        <Loader2 className="size-3.5 animate-spin" aria-hidden />
        {run.attempt && run.attempt > 1 ? `Attempt ${run.attempt}` : "Running"}
        {run.progress?.message ? ` · ${run.progress.message}` : ""}
      </span>
    )
  }
  const { report } = run
  const icon = {
    succeeded: <CircleCheck className="size-3.5 text-emerald-600 dark:text-emerald-400" />,
    failed: <CircleAlert className="size-3.5 text-destructive" />,
    skipped: <SkipForward className="size-3.5 text-muted-foreground" />,
    cancelled: <CircleSlash className="size-3.5 text-muted-foreground" />,
  }[report.status]
  return (
    <span className="flex items-center gap-1 text-xs text-muted-foreground">
      {icon}
      <span className="capitalize">{report.status}</span>
      {report.status !== "skipped" && report.status !== "cancelled" && (
        <span>· {formatMs(report.durationMs)}</span>
      )}
    </span>
  )
}

/** A workflow node on the canvas: icon, title, summary, ports and problems. */
export function NodeView({ data, selected }: NodeProps<FlowNode>) {
  const ports = outputPorts(data.type)
  const errors = data.issues.filter((i) => i.severity === "error")
  const warnings = data.issues.filter((i) => i.severity === "warning")
  const run = data.run
  const status = run?.state === "done" ? run.report.status : run?.state
  return (
    <div
      className={cn(
        "relative w-56 rounded-lg border bg-card text-card-foreground shadow-sm transition-[box-shadow,opacity]",
        selected && "ring-2 ring-ring",
        errors.length > 0 && "border-destructive",
        status === "running" && "border-primary ring-2 ring-primary/40",
        status === "succeeded" && "border-emerald-600/60",
        status === "failed" && "border-destructive ring-2 ring-destructive/30",
        (status === "skipped" || status === "cancelled") && "opacity-60",
      )}
      data-node-type={data.type}
      data-run-state={status}
    >
      {hasInput(data.type) && (
        <Handle
          type="target"
          id="in"
          position={Position.Left}
          className="!size-2.5 !border-2 !border-background !bg-muted-foreground"
        />
      )}
      <div className="flex items-start gap-2 p-3">
        <span
          className={cn(
            "flex size-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground",
            data.type === "action" && "bg-primary/10 text-primary",
            data.type === "trigger" && "bg-emerald-600/10 text-emerald-700 dark:text-emerald-400",
          )}
        >
          <NodeIcon data={data} />
        </span>
        <div className="grid min-w-0 flex-1 gap-0.5">
          <span className="truncate text-sm font-medium">{data.title}</span>
          {data.detail && (
            <span className="truncate text-xs text-muted-foreground">{data.detail}</span>
          )}
          {run && <RunBadge run={run} />}
        </div>
        {errors.length > 0 ? (
          <CircleAlert
            className="size-4 shrink-0 text-destructive"
            aria-label={`${errors.length} problem${errors.length === 1 ? "" : "s"}`}
          />
        ) : (
          warnings.length > 0 && (
            <TriangleAlert
              className="size-4 shrink-0 text-amber-600 dark:text-amber-400"
              aria-label="warning"
            />
          )
        )}
      </div>
      {errors[0] && (
        <p className="border-t px-3 py-1.5 text-xs text-destructive" title={errors[0].message}>
          <span className="line-clamp-2">{errors[0].message}</span>
        </p>
      )}
      {ports.length > 1 && (
        <div className="grid border-t px-3 py-1 text-right text-[0.6875rem] text-muted-foreground">
          {ports.map((port) => (
            <span key={port} className="leading-5">
              {PORT_LABEL[port]}
            </span>
          ))}
        </div>
      )}
      {ports.map((port, index) => (
        <Handle
          key={port}
          type="source"
          id={port}
          position={Position.Right}
          className={cn("!size-2.5 !border-2 !border-background", PORT_CLASS[port])}
          style={
            ports.length > 1
              ? { top: "auto", bottom: `${(ports.length - 1 - index) * 20 + 6}px` }
              : undefined
          }
          aria-label={PORT_LABEL[port]}
        />
      ))}
    </div>
  )
}
