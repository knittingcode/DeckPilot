import "@xyflow/react/dist/style.css"

import {
  Background,
  type Connection,
  Controls,
  type Edge,
  type EdgeChange,
  type FinalConnectionState,
  MarkerType,
  MiniMap,
  type NodeChange,
  ReactFlow,
  type ReactFlowInstance,
  ReactFlowProvider,
  useNodesInitialized,
  useReactFlow,
} from "@xyflow/react"
import {
  Copy,
  LayoutGrid,
  PanelLeft,
  PanelRight,
  Play,
  Redo2,
  Save,
  Trash2,
  Undo2,
} from "lucide-react"
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { VariableScopeProvider } from "@/components/variable-picker/scope-provider"
import { useEditorShortcuts } from "@/hooks/use-editor-shortcuts"
import { useStatusItem } from "@/hooks/use-status"
import { useTitlebar } from "@/hooks/use-titlebar"
import { IS_MAC } from "@/hooks/use-navigation"
import { api, errorMessage } from "@/lib/api"
import { initialValue } from "@/lib/parameters"
import type {
  ActionType,
  GraphEdge,
  GraphIssue,
  GraphPort,
  ModuleInfo,
  ProfileView,
  ProjectScope,
  Workflow,
  WorkflowGraph,
  WorkflowInput,
  WorkflowOutput,
  WorkflowTrigger,
  WorkflowVariable,
} from "@/lib/types"
import { shortcutLabel } from "@/lib/shortcuts"
import { cn } from "@/lib/utils"

import { type CatalogEntry, DRAG_TYPE, actionEntry, flowEntries, moduleGroups } from "./catalog"
import { CatalogPanel } from "./catalog-panel"
import { type Lookup, describeNode } from "./describe"
import { type Panels, loadPanels, loadViewport, savePanels, saveViewport } from "./view-state"
import { commit, history, redo, replace, settle, undo } from "@/lib/history"

import {
  type Clipboard,
  MAX_NODES,
  PORT_LABEL,
  addNode,
  autoLayout,
  connect,
  connectionProblem,
  copy,
  freePosition,
  outputPorts,
  paste,
  reconnect,
  removeEdges,
  removeNodes,
  updateNode,
} from "./graph"
import { IssueList, NodeInspector } from "./inspector"
import { ProjectScopeSetting } from "./project-scope"
import { effectiveProjectScope, selectsProjects } from "./project-scope-model"
import { type FlowNode, type NodeRun, NodeView } from "./node-view"
import { TriggersEditor } from "./triggers-editor"
import { VariablesEditor } from "./variables-editor"
import { WorkflowInterfaceEditor } from "./workflow-interface"

const NODE_TYPES = { workflow: NodeView }

const EDGE_COLOR: Record<GraphPort, string> = {
  out: "var(--color-muted-foreground)",
  error: "var(--color-destructive)",
  true: "#059669",
  false: "#f59e0b",
}

const edgeId = (e: GraphEdge) => `${e.from}:${e.port}:${e.to}`

export interface BuilderProps {
  profileId: string
  view: ProfileView
  types: ActionType[]
  modules: ModuleInfo[]
  /** The workflow as stored; its graph may come from a conversion. */
  initial: Workflow
  graph: WorkflowGraph
  isNew: boolean
  /** Migration notes when the graph was converted from steps. */
  notes: string[]
  /** Run state per node, shown on the canvas (execution visualization). */
  run?: Record<string, NodeRun>
  /** Extra content of the workflow panel, e.g. the run history. */
  workflowPanel?: ReactNode
  /** Shown above the canvas, e.g. the workflow switcher. */
  toolbar?: ReactNode
  canRun: boolean
  /** Why the workflow cannot run (with what to do); shown on the Run button. */
  runBlocked?: string | null
  running: boolean
  onRun: () => void
  onCancelRun?: () => void
  onSaved: (id: string) => void
  onDeleted: () => void
  onActionsChanged: () => Promise<unknown>
}

export function WorkflowBuilder(props: BuilderProps) {
  return (
    <ReactFlowProvider>
      <Builder {...props} />
    </ReactFlowProvider>
  )
}

/**
 * Fits the graph into the canvas, but never below a readable zoom: a larger
 * graph opens at its start node and is panned from there.
 */
function readableFit(instance: ReactFlowInstance<FlowNode>, canvas: HTMLElement | null) {
  const nodes = instance.getNodes()
  if (!canvas || nodes.length === 0) return
  const bounds = instance.getNodesBounds(nodes)
  const { width, height } = canvas.getBoundingClientRect()
  const fit = Math.min(1, (width * 0.85) / bounds.width, (height * 0.85) / bounds.height)
  if (fit >= 0.7) {
    void instance.fitView({ padding: 0.15, maxZoom: 1, duration: 150 })
    return
  }
  const zoom = 0.8
  const start = nodes.find((n) => n.data.type === "trigger") ?? nodes[0]
  void instance.setViewport(
    {
      x: 48 - (start?.position.x ?? bounds.x) * zoom,
      y: height / 2 - (bounds.y + bounds.height / 2) * zoom,
      zoom,
    },
    { duration: 150 },
  )
}

function ToolButton({
  label,
  shortcut,
  disabled,
  onClick,
  children,
}: {
  label: string
  shortcut?: string
  disabled?: boolean
  onClick: () => void
  children: ReactNode
}) {
  return (
    <Button
      variant="ghost"
      size="icon"
      className="size-7"
      aria-label={label}
      title={shortcut ? `${label} (${shortcut})` : label}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </Button>
  )
}

function Builder({
  profileId,
  view,
  types,
  modules,
  initial,
  graph: initialGraph,
  isNew,
  notes,
  run,
  workflowPanel,
  toolbar,
  canRun,
  runBlocked,
  running,
  onRun,
  onCancelRun,
  onSaved,
  onDeleted,
  onActionsChanged,
}: BuilderProps) {
  const flow = useReactFlow<FlowNode>()
  const [name, setName] = useState(initial.name)
  const [description, setDescription] = useState(initial.description ?? "")
  const [inputs, setInputs] = useState<WorkflowInput[]>(initial.inputs ?? [])
  const [outputs, setOutputs] = useState<WorkflowOutput[]>(initial.outputs ?? [])
  const [variables, setVariables] = useState<WorkflowVariable[]>(initial.variables ?? [])
  const [triggers, setTriggers] = useState<WorkflowTrigger[]>(initial.triggers ?? [])
  const [triggersPaused, setTriggersPaused] = useState(Boolean(initial.triggersPaused))
  const [state, setState] = useState(() => history(initialGraph))
  const graph = state.present
  const [selected, setSelected] = useState<string[]>([])
  const [selectedEdges, setSelectedEdges] = useState<string[]>([])
  // Sizes the canvas measured; controlled nodes must carry them.
  const [sizes, setSizes] = useState<Record<string, { width: number; height: number }>>({})
  const [issues, setIssues] = useState<GraphIssue[]>([])
  const [saving, setSaving] = useState(false)
  const [panels, setPanelState] = useState(() => loadPanels(window.innerWidth))
  const setPanels = (update: (current: Panels) => Panels) => {
    setPanelState((current) => {
      const next = update(current)
      savePanels(next)
      return next
    })
  }
  const dragOrigin = useRef<WorkflowGraph | null>(null)
  const canvas = useRef<HTMLDivElement>(null)
  // The first view, once the nodes have their sizes.
  const measured = useNodesInitialized()
  const [ready, setReady] = useState(false)
  const fitted = useRef(false)
  useEffect(() => {
    if (!measured || !ready || fitted.current) return
    fitted.current = true
    // Where the user left this workflow, or a readable first view.
    const saved = loadViewport(initial.id)
    if (saved) void flow.setViewport(saved)
    else readableFit(flow, canvas.current)
  }, [measured, ready, flow, initial.id])
  const clipboard = useRef<Clipboard | null>(null)
  const actions = view.profile.actions
  const [projectScope, setProjectScope] = useState<ProjectScope>(() =>
    effectiveProjectScope(initial, initialGraph, actions, types),
  )

  const lookup = useMemo<Lookup>(
    () => ({
      actions,
      types,
      workflows: view.profile.workflows,
      projects: view.profile.projects,
      modules,
      inputs,
      outputs,
      variables,
      triggers,
      triggersPaused,
    }),
    [
      actions,
      types,
      view.profile.workflows,
      view.profile.projects,
      modules,
      inputs,
      outputs,
      variables,
      triggers,
      triggersPaused,
    ],
  )
  const loopTargets = view.profile.workflows
    .filter((w) => w.id !== initial.id)
    .map((w) => ({ id: w.id, name: w.name }))
  const loopTarget = loopTargets[0]?.id
  const groups = useMemo(
    () => [
      {
        id: "flow",
        title: "Flow",
        entries: flowEntries(loopTarget),
        types: [],
      },
      ...moduleGroups(modules, actions),
    ],
    [modules, actions, loopTarget],
  )

  const workflow: Workflow = useMemo(
    () => ({
      ...initial,
      name: name.trim(),
      description: description.trim(),
      graph,
      projectScope,
      inputs: inputs.length ? inputs : undefined,
      outputs: outputs.length ? outputs : undefined,
      variables: variables.length ? variables : undefined,
      triggers: triggers.length ? triggers : undefined,
      triggersPaused: triggersPaused || undefined,
    }),
    [
      initial,
      name,
      description,
      graph,
      projectScope,
      inputs,
      outputs,
      variables,
      triggers,
      triggersPaused,
    ],
  )
  const dirty =
    isNew ||
    notes.length > 0 ||
    JSON.stringify({
      n: workflow.name,
      d: workflow.description ?? "",
      g: graph,
      s: projectScope,
      i: inputs,
      o: outputs,
      v: variables,
      t: triggers,
      p: triggersPaused,
    }) !==
      JSON.stringify({
        n: initial.name,
        d: initial.description ?? "",
        g: initial.graph,
        s: initial.projectScope ?? projectScope,
        i: initial.inputs ?? [],
        o: initial.outputs ?? [],
        v: initial.variables ?? [],
        t: initial.triggers ?? [],
        p: Boolean(initial.triggersPaused),
      })

  // The engine's validation, shown on the nodes.
  useEffect(() => {
    let current = true
    const timer = setTimeout(() => {
      api.validateWorkflowGraph(profileId, workflow).then(
        (found) => {
          if (current) setIssues(found)
        },
        (error: unknown) => {
          if (current) setIssues([{ severity: "error", message: errorMessage(error) }])
        },
      )
    }, 200)
    return () => {
      current = false
      clearTimeout(timer)
    }
  }, [profileId, workflow])

  const errors = issues.filter((i) => i.severity === "error")
  const graphIssues = issues.filter((i) => !i.node)
  const canSave = dirty && errors.length === 0 && workflow.name.length > 0 && !saving

  const change = useCallback((next: WorkflowGraph) => {
    setState((s) => commit(s, next))
  }, [])

  const save = useCallback(() => {
    if (!canSave) return
    setSaving(true)
    api
      .saveWorkflow(profileId, workflow)
      .then((saved) => {
        toast.success("Workflow saved")
        onSaved(saved.id)
      })
      .catch((e: unknown) =>
        toast.error("The workflow was not saved", { description: errorMessage(e) }),
      )
      .finally(() => {
        setSaving(false)
      })
  }, [canSave, profileId, workflow, onSaved])

  const selectOnly = (ids: string[]) => {
    setSelected(ids)
    setSelectedEdges([])
  }

  /** Adds a catalog entry; after the selected node when one is selected. */
  const addEntry = (entry: CatalogEntry, position?: { x: number; y: number }) => {
    if (graph.nodes.length >= MAX_NODES) {
      toast.error(`A workflow can have at most ${MAX_NODES} nodes`)
      return
    }
    const after = selected.length === 1 ? graph.nodes.find((n) => n.id === selected[0]) : undefined
    const at =
      position ??
      (after
        ? { x: after.position.x + 280, y: after.position.y }
        : flow.screenToFlowPosition({
            x: window.innerWidth / 2,
            y: window.innerHeight / 2,
          }))
    const added = addNode(graph, entry.kind, freePosition(graph, at), entry.name)
    let next = added.graph
    if (after && !position) {
      const port = outputPorts(after.type).find(
        (p) => connectionProblem(next, { from: after.id, port: p, to: added.id }) === null,
      )
      if (port) next = connect(next, { from: after.id, port, to: added.id })
    }
    change(next)
    selectOnly([added.id])
  }

  const createAction = (type: ActionType) => {
    const parameters = Object.fromEntries(type.parameters.map((p) => [p.key, initialValue(p)]))
    api
      .createAction(profileId, {
        name: type.name,
        actionType: type.id,
        parameters,
        description: "",
        icon: type.node?.icon ?? null,
        label: null,
      })
      .then(async (action) => {
        await onActionsChanged()
        addEntry(actionEntry(action, type))
        toast.success(`Action “${action.name}” created`, {
          description: "Configure it in the Action Library or override parameters here.",
        })
      })
      .catch((e: unknown) => {
        toast.error("The action was not created", {
          description: `${errorMessage(e)} You can create it in the Action Library instead.`,
        })
      })
  }

  const doUndo = () => {
    setState(undo)
  }
  const doRedo = () => {
    setState(redo)
  }
  const doCopy = () => {
    const copied = copy(graph, selected)
    if (copied) clipboard.current = copied
  }
  const doPaste = () => {
    if (!clipboard.current) return
    const pasted = paste(graph, clipboard.current)
    change(pasted.graph)
    selectOnly(pasted.ids)
  }
  const doLayout = () => {
    change(autoLayout(graph))
    window.requestAnimationFrame(() => {
      readableFit(flow, canvas.current)
    })
  }

  // Keyboard shortcuts of the editor (deleting is handled by the canvas).
  useEditorShortcuts({ undo: doUndo, redo: doRedo, copy: doCopy, paste: doPaste, save })

  useTitlebar({
    actions: [
      {
        id: "workflows.save",
        label: "Save",
        icon: Save,
        placement: "primary" as const,
        order: 1,
        disabled: !canSave,
        busy: saving,
        onSelect: save,
        hint: !dirty
          ? "No unsaved changes"
          : errors.length > 0
            ? `Fix ${errors.length} problem${errors.length === 1 ? "" : "s"} to save`
            : workflow.name
              ? `Save (${shortcutLabel("S")})`
              : "Give the workflow a name",
      },
      {
        id: "workflows.run",
        label: running ? "Cancel run" : "Run workflow",
        icon: Play,
        placement: "primary" as const,
        order: 2,
        disabled: running ? !onCancelRun : !canRun || dirty,
        hint: running
          ? "Stop before the next node"
          : dirty
            ? "Save first; runs in the agent for the active profile"
            : canRun
              ? `Run “${initial.name}” in the agent`
              : (runBlocked ?? "Runs in the agent for the active profile"),
        onSelect: running ? onCancelRun : onRun,
      },
    ],
  })

  useStatusItem(
    errors.length > 0
      ? {
          id: "workflows.problems",
          owner: "core",
          placement: "left",
          priority: 60,
          label: `${errors.length} problem${errors.length === 1 ? "" : "s"}`,
          tooltip: "Problems in the open workflow; it cannot be saved until they are fixed",
          tone: "danger",
        }
      : null,
  )

  const nodes: FlowNode[] = useMemo(
    () =>
      graph.nodes.map((node) => {
        const text = describeNode(node, lookup)
        const action = node.type === "action" ? actions.find((a) => a.id === node.action) : null
        const type = action ? types.find((t) => t.id === action.actionType) : undefined
        return {
          id: node.id,
          type: "workflow",
          position: node.position,
          measured: sizes[node.id],
          selected: selected.includes(node.id),
          deletable: node.type !== "trigger",
          data: {
            type: node.type,
            title: text.title,
            detail: text.detail,
            icon: action?.icon ?? type?.node?.icon ?? null,
            issues: issues.filter((i) => i.node === node.id),
            run: run?.[node.id],
          },
        }
      }),
    [graph.nodes, lookup, actions, types, selected, issues, run, sizes],
  )
  const edges: Edge[] = useMemo(
    () =>
      graph.edges.map((edge) => {
        const id = edgeId(edge)
        const taken =
          run?.[edge.from]?.state === "done" &&
          (run[edge.to]?.state === "running" ||
            (run[edge.to]?.state === "done" &&
              (run[edge.to] as { report: { status: string } }).report.status !== "skipped"))
        return {
          id,
          source: edge.from,
          sourceHandle: edge.port,
          target: edge.to,
          targetHandle: "in",
          selected: selectedEdges.includes(id),
          label: edge.port === "out" ? undefined : PORT_LABEL[edge.port],
          animated: run?.[edge.to]?.state === "running",
          style: { stroke: EDGE_COLOR[edge.port], strokeWidth: taken ? 2.5 : 1.5 },
          markerEnd: { type: MarkerType.ArrowClosed, color: EDGE_COLOR[edge.port] },
          labelBgPadding: [4, 2] as [number, number],
          labelStyle: { fontSize: 11 },
        }
      }),
    [graph.edges, selectedEdges, run],
  )

  const onNodesChange = (changes: NodeChange<FlowNode>[]) => {
    let next = graph
    let selection: string[] | null = null
    let settled = false
    let moved = false
    const measuredNow: Record<string, { width: number; height: number }> = {}
    for (const c of changes) {
      if (c.type === "dimensions" && c.dimensions) measuredNow[c.id] = c.dimensions
      if (c.type === "position" && c.position) {
        const position = c.position
        if (c.dragging && !dragOrigin.current) dragOrigin.current = graph
        next = updateNode(next, c.id, (n) => ({ ...n, position }))
        moved = true
      }
      if (c.type === "position" && c.dragging === false) settled = true
      if (c.type === "select") {
        selection ??= [...selected]
        selection = c.selected
          ? [...new Set([...selection, c.id])]
          : selection.filter((id) => id !== c.id)
      }
    }
    if (Object.keys(measuredNow).length > 0) setSizes((current) => ({ ...current, ...measuredNow }))
    const removed = changes.filter((c) => c.type === "remove").map((c) => c.id)
    if (removed.length > 0) {
      change(removeNodes(next, removed))
      setSelected([])
      return
    }
    if (selection) setSelected(selection)
    if (moved && !dragOrigin.current) {
      // Moved without dragging (arrow keys): one undo step per move.
      change(next)
      return
    }
    if (next !== graph) setState((s) => replace(s, next))
    if (settled && dragOrigin.current) {
      const origin = dragOrigin.current
      dragOrigin.current = null
      setState((s) => settle(s, origin))
    }
  }

  const onEdgesChange = (changes: EdgeChange[]) => {
    const removed = changes.filter((c) => c.type === "remove").map((c) => c.id)
    if (removed.length > 0) {
      change(
        removeEdges(
          graph,
          graph.edges.filter((e) => removed.includes(edgeId(e))),
        ),
      )
    }
    let selection = selectedEdges
    for (const c of changes) {
      if (c.type === "select") {
        selection = c.selected ? [...selection, c.id] : selection.filter((id) => id !== c.id)
      }
    }
    if (selection !== selectedEdges) setSelectedEdges(selection)
  }

  const toEdge = (c: Connection | Edge): GraphEdge => ({
    from: c.source,
    port: (c.sourceHandle ?? "out") as GraphPort,
    to: c.target,
  })
  const onConnect = (connection: Connection) => {
    const edge = toEdge(connection)
    const problem = connectionProblem(graph, edge)
    if (problem) toast.error(problem)
    else change(connect(graph, edge))
  }
  // A connection dropped where it cannot go: say why.
  const onConnectEnd = (_: MouseEvent | TouchEvent, state: FinalConnectionState) => {
    if (state.isValid !== false || !state.toNode) return
    const [from, to, port] =
      state.fromHandle.type === "target"
        ? [state.toNode.id, state.fromNode.id, state.toHandle?.id ?? "out"]
        : [state.fromNode.id, state.toNode.id, state.fromHandle.id ?? "out"]
    const problem = connectionProblem(graph, { from, port: port as GraphPort, to })
    if (problem) toast.error(problem)
  }
  const onReconnect = (old: Edge, connection: Connection) => {
    const result = reconnect(graph, toEdge(old), toEdge(connection))
    if (result.problem) toast.error(result.problem)
    else change(result.graph)
  }

  const onDrop = (event: React.DragEvent) => {
    const key = event.dataTransfer.getData(DRAG_TYPE)
    const entry = groups.flatMap((g) => g.entries).find((e) => e.key === key)
    if (!entry) return
    event.preventDefault()
    addEntry(entry, flow.screenToFlowPosition({ x: event.clientX, y: event.clientY }))
  }

  const node = selected.length === 1 ? graph.nodes.find((n) => n.id === selected[0]) : undefined
  // Outputs of the shown run, for mapping previews.
  const runValues = Object.fromEntries(
    Object.entries(run ?? {}).flatMap(([id, state]) =>
      state.state === "done" && state.report.output ? [[id, state.report.output]] : [],
    ),
  )

  return (
    <div className="flex h-[calc(100svh-var(--titlebar-height)-var(--statusbar-height)-3rem)] min-h-[28rem] flex-col overflow-hidden rounded-lg border bg-background">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b px-2">
        <Button
          variant="ghost"
          size="icon"
          className="size-7"
          aria-label={panels.catalog ? "Hide nodes" : "Show nodes"}
          aria-pressed={panels.catalog}
          onClick={() => {
            setPanels((p) => ({ ...p, catalog: !p.catalog }))
          }}
        >
          <PanelLeft />
        </Button>
        <div className="flex min-w-0 flex-1 items-center gap-2">{toolbar}</div>
        <div className="flex items-center" role="toolbar" aria-label="Edit">
          <ToolButton
            label="Undo"
            shortcut={shortcutLabel("Z")}
            disabled={state.past.length === 0}
            onClick={doUndo}
          >
            <Undo2 />
          </ToolButton>
          <ToolButton
            label="Redo"
            shortcut={shortcutLabel("Z", true)}
            disabled={state.future.length === 0}
            onClick={doRedo}
          >
            <Redo2 />
          </ToolButton>
          <ToolButton label="Arrange nodes" onClick={doLayout}>
            <LayoutGrid />
          </ToolButton>
        </div>
        <span className="text-xs text-muted-foreground" aria-live="polite">
          {dirty ? "Unsaved changes" : "Saved"}
        </span>
        <Button
          variant="ghost"
          size="icon"
          className="size-7"
          aria-label={panels.inspector ? "Hide inspector" : "Show inspector"}
          aria-pressed={panels.inspector}
          onClick={() => {
            setPanels((p) => ({ ...p, inspector: !p.inspector }))
          }}
        >
          <PanelRight />
        </Button>
      </div>
      <div className="flex min-h-0 flex-1">
        {panels.catalog && (
          <aside className="flex w-56 shrink-0 flex-col border-r" aria-label="Node catalog">
            <CatalogPanel
              groups={groups}
              onAdd={(entry) => {
                addEntry(entry)
              }}
              onCreateAction={createAction}
            />
          </aside>
        )}
        <div
          className="relative min-w-0 flex-1"
          onDragOver={(event) => {
            if (event.dataTransfer.types.includes(DRAG_TYPE)) {
              event.preventDefault()
              event.dataTransfer.dropEffect = "copy"
            }
          }}
          onDrop={onDrop}
          ref={canvas}
          data-testid="workflow-canvas"
        >
          <ReactFlow<FlowNode>
            nodes={nodes}
            edges={edges}
            nodeTypes={NODE_TYPES}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            onConnectEnd={onConnectEnd}
            onReconnect={onReconnect}
            onMoveEnd={(_, viewport) => {
              saveViewport(initial.id, viewport)
            }}
            isValidConnection={(c) => connectionProblem(graph, toEdge(c)) === null}
            onInit={() => {
              setReady(true)
            }}
            onPaneClick={() => {
              selectOnly([])
            }}
            deleteKeyCode={["Backspace", "Delete"]}
            multiSelectionKeyCode={IS_MAC ? "Meta" : "Control"}
            minZoom={0.2}
            maxZoom={2}
            colorMode="system"
          >
            <Background gap={20} size={1} />
            <Controls showInteractive={false} position="bottom-left" />
            <MiniMap
              pannable
              zoomable
              position="bottom-right"
              style={{ width: 160, height: 100 }}
              nodeBorderRadius={6}
              bgColor="var(--color-card)"
              maskColor="color-mix(in oklab, var(--color-muted) 70%, transparent)"
              nodeColor={(n) =>
                (n.data as { issues?: unknown[] }).issues?.length
                  ? "var(--color-destructive)"
                  : "var(--color-muted-foreground)"
              }
              className="rounded-md border"
            />
          </ReactFlow>
        </div>
        {panels.inspector && (
          // References in this workflow resolve in its runs, which have their
          // own project: the picker never shows the active project as theirs.
          <VariableScopeProvider
            context={{ workflow: initial.id, projectSource: "execution" }}
            names={{ workflow: name.trim() || initial.name }}
          >
            <aside
              className="w-68 shrink-0 overflow-x-hidden overflow-y-auto border-l"
              aria-label="Inspector"
            >
              {node ? (
                <NodeInspector
                  graph={graph}
                  node={node}
                  issues={issues.filter((i) => i.node === node.id)}
                  lookup={lookup}
                  projects={view.profile.projects}
                  loopTargets={loopTargets}
                  onChange={(update) => {
                    change(updateNode(graph, node.id, update))
                  }}
                  onDelete={() => {
                    change(removeNodes(graph, [node.id]))
                    selectOnly([])
                  }}
                  run={run?.[node.id]}
                  values={runValues}
                />
              ) : (
                <div className="grid">
                  <div className="grid gap-3 px-4 py-3">
                    <h2 className="text-sm font-semibold">Workflow</h2>
                    <div className="grid gap-1.5">
                      <Label htmlFor="workflow-name" className="text-xs">
                        Name
                      </Label>
                      <Input
                        id="workflow-name"
                        value={name}
                        onChange={(e) => {
                          setName(e.target.value)
                        }}
                      />
                    </div>
                    <div className="grid gap-1.5">
                      <Label htmlFor="workflow-description" className="text-xs">
                        Description
                      </Label>
                      <Textarea
                        id="workflow-description"
                        rows={2}
                        value={description}
                        onChange={(e) => {
                          setDescription(e.target.value)
                        }}
                      />
                    </div>
                    <ProjectScopeSetting
                      scope={projectScope}
                      selectsProjects={selectsProjects(graph, actions, types)}
                      onChange={setProjectScope}
                    />
                    <IssueList issues={graphIssues} />
                    {errors.length > 0 && graphIssues.length === 0 && (
                      <p className="text-xs text-destructive">
                        {errors.length} problem{errors.length === 1 ? "" : "s"} on nodes; select a
                        node to see it.
                      </p>
                    )}
                    {selected.length > 1 && (
                      <p className="text-xs text-muted-foreground">
                        {selected.length} nodes selected. Copy with {shortcutLabel("C")}, delete
                        with ⌫.
                      </p>
                    )}
                    {notes.length > 0 && (
                      <div className="grid gap-1 rounded-md border border-amber-500/40 bg-amber-500/10 p-2.5 text-xs">
                        <p className="font-medium">Converted from steps</p>
                        <p className="text-muted-foreground">
                          Review these differences, then save. The steps are kept.
                        </p>
                        <ul className="list-disc pl-4">
                          {notes.map((note) => (
                            <li key={note}>{note}</li>
                          ))}
                        </ul>
                      </div>
                    )}
                    {!isNew && (
                      <div className="flex flex-wrap gap-2">
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => {
                            api
                              .duplicateWorkflow(profileId, initial.id, `${initial.name} (copy)`)
                              .then((copy) => {
                                toast.success("Workflow duplicated")
                                onSaved(copy.id)
                              })
                              .catch((e: unknown) =>
                                toast.error("The workflow was not duplicated", {
                                  description: errorMessage(e),
                                }),
                              )
                          }}
                        >
                          <Copy /> Duplicate
                        </Button>
                        <ConfirmDialog
                          trigger={
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled={(view.workflowUsages[initial.id] ?? []).length > 0}
                              title={
                                (view.workflowUsages[initial.id] ?? []).length > 0
                                  ? "Used by a layout or another workflow"
                                  : undefined
                              }
                            >
                              <Trash2 /> Delete
                            </Button>
                          }
                          title={`Delete “${initial.name}”?`}
                          description="The workflow and its graph are removed from this profile."
                          onConfirm={() => {
                            api
                              .deleteWorkflow(profileId, initial.id)
                              .then(() => {
                                toast.success("Workflow deleted")
                                onDeleted()
                              })
                              .catch((e: unknown) =>
                                toast.error("The workflow was not deleted", {
                                  description: errorMessage(e),
                                }),
                              )
                          }}
                        />
                      </div>
                    )}
                  </div>
                  <WorkflowInterfaceEditor
                    inputs={inputs}
                    outputs={outputs}
                    onInputs={setInputs}
                    onOutputs={setOutputs}
                  />
                  <VariablesEditor variables={variables} onChange={setVariables} />
                  <TriggersEditor
                    triggers={triggers}
                    paused={triggersPaused}
                    inputs={inputs}
                    onTriggers={setTriggers}
                    onPaused={setTriggersPaused}
                  />
                  {workflowPanel}
                </div>
              )}
            </aside>
          </VariableScopeProvider>
        )}
      </div>
      <p className={cn("sr-only")} aria-live="polite">
        {errors.length > 0 ? `${errors.length} problems` : "No problems"}
      </p>
    </div>
  )
}
