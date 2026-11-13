// Targets offered by the library, the inspector and the context menu.

import type { BindingTarget, DeviceLayout, Profile } from "@/lib/types"

export interface TargetOption {
  value: string
  label: string
  target: BindingTarget
  kind: "action" | "workflow" | "navigation" | "stateful"
  icon?: string | null
}

export interface TargetGroup {
  label: string
  options: TargetOption[]
}

export function encodeTarget(target: BindingTarget | undefined): string {
  if (!target) return "none"
  switch (target.type) {
    case "action":
      return `action:${target.action}`
    case "workflow":
      return `workflow:${target.workflow}`
    case "openPage":
      return `page:${target.page}`
    case "back":
      return "back"
    case "stateful":
      return `stateful:${target.machine}`
  }
}

export function decodeTarget(value: string): BindingTarget | null {
  if (value === "back") return { type: "back" }
  const split = value.indexOf(":")
  if (split < 0) return null
  const [kind, id] = [value.slice(0, split), value.slice(split + 1)]
  if (kind === "action") return { type: "action", action: id }
  if (kind === "workflow") return { type: "workflow", workflow: id }
  if (kind === "page") return { type: "openPage", page: id }
  if (kind === "stateful" && id === "workSession") return { type: "stateful", machine: id }
  return null
}

/** Everything that can be placed on a control of `pageId`, grouped. */
export function targetGroups(
  profile: Pick<Profile, "actions" | "workflows">,
  layout: DeviceLayout,
  pageId: string,
): TargetGroup[] {
  const option = (target: BindingTarget, label: string, kind: TargetOption["kind"]) => ({
    value: encodeTarget(target),
    label,
    target,
    kind,
  })
  const groups: TargetGroup[] = [
    {
      label: "Actions",
      options: profile.actions.map((a) => ({
        ...option({ type: "action", action: a.id }, a.name, "action"),
        icon: a.icon,
      })),
    },
    {
      label: "Workflows",
      options: profile.workflows.map((w) =>
        option({ type: "workflow", workflow: w.id }, w.name, "workflow"),
      ),
    },
    {
      label: "Navigation",
      options: [
        option({ type: "back" }, "Back", "navigation"),
        ...layout.pages
          .filter((p) => p.id !== pageId)
          .map((p) => option({ type: "openPage", page: p.id }, `Open ${p.name}`, "navigation")),
      ],
    },
  ]
  return groups.filter((g) => g.options.length > 0)
}

/**
 * Targets offered only on gesture rows (single click, double click, long
 * press), after `targetGroups`: the Work Session controller, a stateful key
 * with the machine's default rules. It needs no action instance; the agent
 * asks the work session for the operation its state and the gesture pick.
 */
export function gestureOnlyGroups(): TargetGroup[] {
  const target: BindingTarget = { type: "stateful", machine: "workSession" }
  return [
    {
      label: "Work Session",
      options: [
        {
          value: encodeTarget(target),
          label: "Work Session controller",
          target,
          kind: "stateful",
        },
      ],
    },
  ]
}
