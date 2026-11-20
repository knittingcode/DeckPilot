// Targets offered by the library, the inspector and the context menu.

import { i18n } from "@/i18n"
import type { BindingTarget, DeviceLayout, Profile } from "@/lib/types"

export interface TargetOption {
  value: string
  label: string
  target: BindingTarget
  kind: "action" | "workflow" | "navigation" | "stateful"
  icon?: string | null
  /**
   * Why it cannot be placed now (a plugin action whose plugin is disabled
   * or not installed). Controls that already use it keep it.
   */
  unavailable?: string
  /** For an action: its type, so compatibility can be checked. */
  actionType?: string
  /**
   * Why it cannot go on the control at hand (docs/device-controls.md): the
   * action does not work on that kind of control or with any of its inputs.
   */
  incompatible?: string
}

export interface TargetGroup {
  /** Stable identity of the group; `label` is in the window's language. */
  id: "actions" | "workflows" | "navigation" | "workSession"
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
  /** Why an action of this type cannot be placed now; `null` when it can. */
  unavailable: (actionType: string) => string | null = () => null,
): TargetGroup[] {
  const option = (target: BindingTarget, label: string, kind: TargetOption["kind"]) => ({
    value: encodeTarget(target),
    label,
    target,
    kind,
  })
  const groups: TargetGroup[] = [
    {
      id: "actions",
      label: i18n.t("editor:targets.actions"),
      options: profile.actions.map((a) => {
        const why = unavailable(a.actionType)
        return {
          ...option({ type: "action", action: a.id }, a.name, "action"),
          icon: a.icon,
          actionType: a.actionType,
          ...(why ? { unavailable: why } : {}),
        }
      }),
    },
    {
      id: "workflows",
      label: i18n.t("editor:targets.workflows"),
      options: profile.workflows.map((w) =>
        option({ type: "workflow", workflow: w.id }, w.name, "workflow"),
      ),
    },
    {
      id: "navigation",
      label: i18n.t("editor:targets.navigation"),
      options: [
        option({ type: "back" }, i18n.t("editor:targets.back"), "navigation"),
        ...layout.pages
          .filter((p) => p.id !== pageId)
          .map((p) =>
            option(
              { type: "openPage", page: p.id },
              i18n.t("editor:targets.openPage", { page: p.name }),
              "navigation",
            ),
          ),
      ],
    },
  ]
  return groups.filter((g) => g.options.length > 0)
}

/**
 * The groups with `incompatible` set on each option `why` explains.
 * Options without an action type (workflows, pages, Back) go on any
 * control.
 */
export function markIncompatible(
  groups: TargetGroup[],
  why: (actionType: string) => string | null,
): TargetGroup[] {
  return groups.map((group) => ({
    ...group,
    options: group.options.map((option) => {
      const reason = option.actionType ? why(option.actionType) : null
      return reason ? { ...option, incompatible: reason } : option
    }),
  }))
}

/** Why the option of `value` cannot be placed now, if it cannot. */
export function unavailableTarget(groups: TargetGroup[], value: string): string | undefined {
  for (const group of groups) {
    const found = group.options.find((o) => o.value === value)
    if (found) return found.unavailable
  }
  return undefined
}

/** The module that runs the Work Session controller. */
export const WORK_SESSION_MODULE = "work-session"

/**
 * Targets offered only on gesture rows (single click, double click, long
 * press), after `targetGroups`: the Work Session controller, a stateful key
 * with the machine's default rules. It needs no action instance; the agent
 * asks the work session for the operation its state and the gesture pick.
 */
export function gestureOnlyGroups(workSession = true): TargetGroup[] {
  // Only while the Work Session module is there to run it.
  if (!workSession) return []
  const target: BindingTarget = { type: "stateful", machine: "workSession" }
  return [
    {
      id: "workSession",
      label: i18n.t("editor:targets.workSession"),
      options: [
        {
          value: encodeTarget(target),
          label: i18n.t("editor:targets.workSessionController"),
          target,
          kind: "stateful",
        },
      ],
    },
  ]
}
