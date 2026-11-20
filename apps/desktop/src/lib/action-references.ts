import type { FocusTarget } from "./focus"
import { controlLabel, triggerLabel } from "./layout-model"
import type { ActionReference, Profile, ReferenceSite } from "./types"
import { i18n } from "@/i18n"
import { describeTrigger } from "@/workflow-builder/trigger-model"

/** A stable identity of the place a site names. */
export function siteKey(site: ReferenceSite): string {
  return JSON.stringify(site)
}

/**
 * Every place in `profile` that uses action `id`: layout bindings, workflow
 * steps and graph nodes, and the triggers of the workflows that use it.
 * The same list, in the same order, as deckpilot_storage's
 * `references::action_references`, computed from the profile at hand.
 */
export function actionReferences(profile: Profile, id: string): ActionReference[] {
  const references: ActionReference[] = []
  for (const layout of profile.layouts) {
    for (const page of layout.pages) {
      for (const binding of page.bindings) {
        if (binding.target.type !== "action" || binding.target.action !== id) continue
        references.push({
          site: {
            type: "binding",
            layout: layout.id,
            page: page.id,
            control: binding.control,
            trigger: binding.trigger,
          },
          ownerName: layout.name,
          detail: page.name,
        })
      }
    }
  }
  for (const workflow of profile.workflows) {
    const before = references.length
    workflow.steps.forEach((step, index) => {
      if (step.action === id) {
        references.push({
          site: { type: "step", workflow: workflow.id, index },
          ownerName: workflow.name,
        })
      }
    })
    for (const node of workflow.graph?.nodes ?? []) {
      if (node.type === "action" && node.action === id) {
        references.push({
          site: { type: "node", workflow: workflow.id, node: node.id },
          ownerName: workflow.name,
          ...(node.label ? { detail: node.label } : {}),
        })
      }
    }
    if (references.length > before) {
      ;(workflow.triggers ?? []).forEach((trigger, index) => {
        references.push({
          site: { type: "trigger", workflow: workflow.id, index },
          ownerName: workflow.name,
          ...(workflow.triggersPaused ? { detail: "paused" } : {}),
          trigger,
        })
      })
    }
  }
  return references
}

/** Whether replacing or unbinding acts on the site itself (not a trigger). */
export function isDirect(reference: ActionReference): boolean {
  return reference.site.type !== "trigger"
}

/** Where a reference is, in the window's language. */
export function describeReference(reference: ActionReference): string {
  const { site, ownerName, detail } = reference
  switch (site.type) {
    case "binding": {
      const values = { layout: ownerName, page: detail ?? "", control: controlLabel(site.control) }
      return site.trigger.type === "press"
        ? i18n.t("actions:referenceSite.binding", values)
        : i18n.t("actions:referenceSite.bindingTrigger", {
            ...values,
            trigger: triggerLabel(site.trigger),
          })
    }
    case "step":
      return i18n.t("actions:referenceSite.step", { workflow: ownerName, step: site.index + 1 })
    case "node":
      return i18n.t("actions:referenceSite.node", {
        workflow: ownerName,
        node: detail ?? site.node,
      })
    case "trigger": {
      const values = {
        workflow: ownerName,
        trigger: reference.trigger
          ? describeTrigger(reference.trigger)
          : i18n.t("actions:referenceSite.aTrigger"),
      }
      return detail === "paused"
        ? i18n.t("actions:referenceSite.triggerPaused", values)
        : i18n.t("actions:referenceSite.trigger", values)
    }
  }
}

/** The page that shows a reference and what it should open. */
export function referenceTarget(reference: ActionReference): {
  page: "workflows" | "designer"
  focus: FocusTarget
} {
  const { site } = reference
  return site.type === "binding"
    ? { page: "designer", focus: { type: "layout", layout: site.layout, page: site.page } }
    : { page: "workflows", focus: { type: "workflow", workflow: site.workflow } }
}
