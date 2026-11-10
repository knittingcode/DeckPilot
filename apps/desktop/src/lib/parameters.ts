import type { ActionParameters, ParameterSpec, ParameterValue } from "./types"

/** Initial form value for a parameter: its declared default, or an empty value of the right type. */
export function initialValue(spec: ParameterSpec): ParameterValue {
  if (spec.default) return spec.default
  switch (spec.kind.type) {
    case "text":
      return { type: "text", value: "" }
    case "integer":
      return { type: "integer", value: spec.kind.min ?? 0 }
    case "boolean":
      return { type: "boolean", value: false }
    case "choice":
      return { type: "choice", value: spec.kind.options[0] ?? "" }
    case "resource":
      return { type: "resource", value: { type: "projectRole", role: "" } }
  }
}

function isEmpty(value: ParameterValue): boolean {
  switch (value.type) {
    case "text":
    case "choice":
      return value.value.trim() === ""
    case "resource":
      return value.value.type === "fixed"
        ? value.value.key.trim() === ""
        : value.value.role.trim() === ""
    case "integer":
    case "boolean":
      return false
  }
}

/**
 * Builds the parameter map sent to the backend. Empty optional values are
 * omitted so that backend defaults apply; empty required values are kept so
 * that the backend reports them as missing.
 */
export function buildParameters(
  specs: ParameterSpec[],
  values: Record<string, ParameterValue>,
): ActionParameters {
  const result: ActionParameters = {}
  for (const spec of specs) {
    const value = values[spec.key]
    if (!value || (!spec.required && isEmpty(value))) continue
    result[spec.key] = value
  }
  return result
}

/** Short human-readable representation of a parameter value. */
export function describeValue(value: ParameterValue): string {
  switch (value.type) {
    case "text":
    case "choice":
      return value.value
    case "integer":
      return String(value.value)
    case "boolean":
      return value.value ? "yes" : "no"
    case "resource":
      return value.value.type === "fixed" ? value.value.key : `active project › ${value.value.role}`
  }
}
