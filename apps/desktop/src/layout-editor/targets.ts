// Targets offered by the library, the inspector and the context menu.

import type { BindingTarget, DeviceLayout, Profile } from "@/lib/types"

export interface TargetOption {
  value: string
  label: string
  target: BindingTarget
  kind: "action" | "workflow" | "navigation"
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
 * press), after `targetGroups`. This is the slot for the Work Session
 * controller (workstream G4): a "Work Session controller" group whose
 * option encodes the stateful `BindingTarget` once the contract in
 * `lib/types.ts` has one, with `encodeTarget`/`decodeTarget` extended to
 * match. Until then there are none.
 */
export function gestureOnlyGroups(): TargetGroup[] {
  return []
}
