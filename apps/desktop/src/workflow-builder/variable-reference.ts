/**
 * References to shared, system and workflow variables in node mappings
 * (`{{ shared.paths.root }}`, `{{ system.project.current.key }}`), mirroring
 * `deckpilot_core::variables::VariableRef::parse` and
 * `VariableType::assignable_to`. Only metadata: values are resolved by the
 * agent when the node runs (and previewed by the variable picker).
 */
import { type ReactNode, createElement } from "react"

import { VariablePicker as UniversalVariablePicker } from "@/components/variable-picker/variable-picker"
import { type TranslationKey, i18n } from "@/i18n"
import type { ParameterKind, VariableDefinition, VariableType } from "@/lib/types"
import { TEMPLATE_KIND } from "@/lib/variables"

/**
 * Labels translated when they are read, in the window's language: for
 * label records that callers index like plain strings. Components that
 * show them use `useTranslation`, so they re-render on a language change.
 */
export function translatedLabels<K extends string>(
  keys: Record<K, TranslationKey>,
): Readonly<Record<K, string>> {
  const labels = {} as Record<K, string>
  for (const key of Object.keys(keys) as K[]) {
    const translationKey: TranslationKey = keys[key]
    Object.defineProperty(labels, key, { enumerable: true, get: () => i18n.t(translationKey) })
  }
  return labels
}

const SYSTEM_LABEL = translatedLabels({
  "system.project.current.id": "workflows:variables.system.projectId",
  "system.project.current.key": "workflows:variables.system.projectKey",
  "system.project.current.name": "workflows:variables.system.projectName",
  "system.profile.current.id": "workflows:variables.system.profileId",
  "system.profile.current.name": "workflows:variables.system.profileName",
  "system.device.current.id": "workflows:variables.system.deviceId",
  "system.platform": "workflows:variables.system.platform",
  "workflow.id": "workflows:variables.system.workflowId",
  "workflow.execution.id": "workflows:variables.system.executionId",
})

const SYSTEM_TYPE: Record<keyof typeof SYSTEM_LABEL, VariableType> = {
  "system.project.current.id": { type: "projectId" },
  "system.project.current.key": { type: "string" },
  "system.project.current.name": { type: "string" },
  "system.profile.current.id": { type: "profileId" },
  "system.profile.current.name": { type: "string" },
  "system.device.current.id": { type: "string" },
  "system.platform": { type: "string" },
  "workflow.id": { type: "string" },
  "workflow.execution.id": { type: "string" },
}

/** The read-only variables of a run and their types. */
export const SYSTEM_VARIABLES: Record<string, { type: VariableType; readonly label: string }> =
  Object.fromEntries(
    (Object.keys(SYSTEM_TYPE) as (keyof typeof SYSTEM_TYPE)[]).map((reference) => [
      reference,
      {
        type: SYSTEM_TYPE[reference],
        get label() {
          return SYSTEM_LABEL[reference]
        },
      },
    ]),
  )

const SEGMENT = /^[a-z][A-Za-z0-9_]{0,31}$/
const STEP = /^steps\.[a-z][a-z0-9_]{0,47}\.output\.[A-Za-z0-9_]{1,64}$/

/** Why `text` (without braces) is not a reference, or `null`. */
export function referenceSyntaxProblem(text: string): string | null {
  const reference = text.trim()
  if (!reference) return i18n.t("workflows:variables.problem.empty")
  if (reference.startsWith("steps.")) {
    return STEP.test(reference)
      ? null
      : i18n.t("workflows:variables.problem.notStep", { reference })
  }
  if (reference.startsWith("shared.")) {
    const key = reference.slice("shared.".length).split(".")
    if (key.length < 2 || key.length > 4 || !key.every((s) => SEGMENT.test(s))) {
      return i18n.t("workflows:variables.problem.notShared", { reference })
    }
    if (["steps", "system", "workflow"].includes(key[0] ?? "")) {
      return i18n.t("workflows:variables.problem.reserved", { namespace: key[0] ?? "" })
    }
    return null
  }
  if (reference in SYSTEM_VARIABLES) return null
  return i18n.t("workflows:variables.problem.unknown", { reference })
}

/** The type of a non-`steps` reference, when known before the run. */
export function referenceType(
  reference: string,
  definitions: readonly VariableDefinition[] = [],
): VariableType | undefined {
  const system = SYSTEM_VARIABLES[reference]
  if (system) return system.type
  if (reference.startsWith("shared.")) {
    const key = reference.slice("shared.".length)
    return definitions.find((d) => d.key === key)?.valueType
  }
  return undefined
}

const isText = (kind: ParameterKind) =>
  kind.type === "text" || kind.type === "lines" || kind.type === "pairs" || kind.type === "code"

/**
 * Whether a whole value of `type` fills a parameter of `kind`, keeping its
 * native type (mirrors `VariableType::assignable_to`).
 */
export function assignable(type: VariableType, kind: ParameterKind): boolean {
  switch (type.type) {
    case "secretRef":
      return kind.type === "credential"
    case "integer":
      return kind.type === "integer" || isText(kind)
    case "boolean":
      return kind.type === "boolean" || isText(kind)
    case "projectId":
      return kind.type === "project" || isText(kind)
    case "string":
      return kind.type === "choice" || isText(kind)
    case "profileId":
      return isText(kind)
    case "resource":
      return kind.type === "resource" && (!type.kind || !kind.kind || type.kind === kind.kind)
    case "json":
      return (kind.type === "code" && kind.language === "json") || kind.type === "text"
  }
}

/** Values that never become text: credential names and resources. */
export function rendersAsText(type: VariableType): boolean {
  return type.type !== "secretRef" && type.type !== "resource"
}

/** What a parameter expects, for messages. */
export function expects(kind: ParameterKind): string {
  switch (kind.type) {
    case "integer":
      return i18n.t("workflows:variables.expects.integer")
    case "boolean":
      return i18n.t("workflows:variables.expects.boolean")
    case "choice":
      return i18n.t("workflows:variables.expects.choice")
    case "resource":
      return i18n.t("workflows:variables.expects.resource")
    case "project":
      return i18n.t("workflows:variables.expects.project")
    case "credential":
      return i18n.t("workflows:variables.expects.credential")
    default:
      return i18n.t("workflows:variables.expects.text")
  }
}

/**
 * Hook for the universal variable picker. The editor renders it beside a
 * reference field; it calls `onPick` with a reference without braces.
 */
export type VariablePicker = (props: {
  /**
   * The kind of the parameter the reference fills, for type filtering;
   * without one the reference is a part of a text template.
   */
  expected?: ParameterKind
  /** The reference field's label, for the picker's accessible names. */
  fieldLabel?: string
  onPick: (reference: string) => void
}) => ReactNode

/**
 * The picker the builder shows beside reference fields: the universal
 * variable picker, in the scope the builder provides (run context and the
 * graph's step outputs). It lists metadata and redacted previews only, is
 * hidden while the agent lacks the `variables` capability, and hands back
 * the reference, which is what the mapping stores.
 */
export const VARIABLE_PICKER: VariablePicker = ({
  expected,
  fieldLabel = i18n.t("workflows:variables.picker.field"),
  onPick,
}) =>
  createElement(UniversalVariablePicker, {
    kind: expected ?? TEMPLATE_KIND,
    fieldLabel,
    label: i18n.t("workflows:variables.picker.label", { field: fieldLabel }),
    className: "size-7",
    onPick,
  })
