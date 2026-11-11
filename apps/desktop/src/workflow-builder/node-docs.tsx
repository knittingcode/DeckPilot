import { CAPABILITIES } from "@/lib/capabilities"
import type { ActionType, ParameterKind } from "@/lib/types"

import type { Lookup } from "./describe"

const KIND_LABEL: Record<ParameterKind["type"], string> = {
  text: "text",
  integer: "number",
  boolean: "yes/no",
  choice: "choice",
  resource: "resource",
  project: "project",
}

function Badge({ children, muted }: { children: string; muted?: boolean }) {
  return (
    <span
      className={
        muted
          ? "rounded border px-1.5 py-0.5 text-[0.6875rem] text-muted-foreground"
          : "rounded bg-muted px-1.5 py-0.5 text-[0.6875rem]"
      }
    >
      {children}
    </span>
  )
}

/**
 * What a node type is, from the module's declarations only: its module and
 * the permissions that module holds, how it behaves (node spec) and its
 * inputs. The editor shows metadata; it never runs module code.
 */
export function NodeDocumentation({ type, lookup }: { type: ActionType; lookup: Lookup }) {
  const module = lookup.modules?.find((m) => m.actionTypes.some((t) => t.id === type.id))
  const spec = type.node
  return (
    <section className="grid gap-2 border-t px-4 py-3" aria-label="About this node">
      <h3 className="text-xs font-medium text-muted-foreground uppercase">About this node</h3>
      <p className="text-xs">
        <code className="font-mono">{type.id}</code>
        {spec && <span className="text-muted-foreground"> · version {spec.version}</span>}
        {module && <span className="text-muted-foreground"> · {module.name} module</span>}
      </p>
      {spec && (
        <div className="flex flex-wrap gap-1">
          <Badge muted={!spec.retrySafe}>{spec.retrySafe ? "Retry-safe" : "Not retry-safe"}</Badge>
          {spec.idempotent && <Badge>Idempotent</Badge>}
          <Badge muted={!spec.cancellable}>
            {spec.cancellable ? "Can be interrupted" : "Cannot be interrupted"}
          </Badge>
          {spec.changesContext && <Badge>Selects a project</Badge>}
        </div>
      )}
      {module && module.capabilities.length > 0 && (
        <div className="grid gap-1">
          <h4 className="text-xs font-medium">Permissions of its module</h4>
          <ul className="grid gap-0.5 text-xs text-muted-foreground">
            {module.capabilities.map((capability) => (
              <li key={capability}>{CAPABILITIES[capability].title}</li>
            ))}
          </ul>
        </div>
      )}
      {type.parameters.length > 0 && (
        <div className="grid gap-1">
          <h4 className="text-xs font-medium">Inputs</h4>
          <ul className="grid gap-0.5 text-xs">
            {type.parameters.map((p) => (
              <li key={p.key} className="flex gap-2">
                <code className="font-mono">{p.key}</code>
                <span className="flex-1 text-muted-foreground">
                  {p.label}
                  {p.required ? "" : " (optional)"}
                </span>
                <span className="text-muted-foreground">{KIND_LABEL[p.kind.type]}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  )
}
