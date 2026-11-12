import { describe, expect, it } from "vitest"

import { buildParameters, describeValue, initialValue } from "./parameters"
import type { ParameterSpec } from "./types"

const text: ParameterSpec = {
  key: "message",
  label: "Message",
  kind: { type: "text", multiline: false },
  required: true,
}
const optionalText: ParameterSpec = { ...text, key: "note", required: false }
const choice: ParameterSpec = {
  key: "direction",
  label: "Direction",
  kind: { type: "choice", options: ["next", "previous"] },
  required: true,
}
const integer: ParameterSpec = {
  key: "count",
  label: "Count",
  kind: { type: "integer", min: 1 },
  required: false,
}

describe("initialValue", () => {
  it("prefers declared defaults", () => {
    const spec = { ...choice, default: { type: "choice", value: "previous" } } as const
    expect(initialValue(spec)).toEqual({ type: "choice", value: "previous" })
  })

  it("derives empty values from the kind", () => {
    expect(initialValue(text)).toEqual({ type: "text", value: "" })
    expect(initialValue(choice)).toEqual({ type: "choice", value: "next" })
    expect(initialValue(integer)).toEqual({ type: "integer", value: 1 })
  })
})

describe("buildParameters", () => {
  it("omits empty optional values but keeps required ones", () => {
    const values = {
      message: { type: "text", value: "" },
      note: { type: "text", value: "  " },
      count: { type: "integer", value: 3 },
    } as const
    expect(buildParameters([text, optionalText, integer], values)).toEqual({
      message: { type: "text", value: "" },
      count: { type: "integer", value: 3 },
    })
  })
})

describe("describeValue", () => {
  it("formats resource references", () => {
    expect(describeValue({ type: "resource", value: { type: "fixed", key: "shop.repo" } })).toBe(
      "shop.repo",
    )
    expect(
      describeValue({ type: "resource", value: { type: "projectRole", role: "repository" } }),
    ).toBe("active project › repository")
  })
})

describe("references", () => {
  it("are described without being resolved", () => {
    expect(describeValue({ type: "reference", value: "shared.projects.favorite" })).toBe(
      "{{ shared.projects.favorite }}",
    )
    expect(describeValue({ type: "template", value: "cd {{ shared.paths.root }}" })).toBe(
      "cd {{ shared.paths.root }}",
    )
  })

  it("count as values of required parameters", () => {
    const values = {
      message: { type: "template", value: "{{ system.project.current.key }}" },
      note: { type: "reference", value: "" },
    } as const
    expect(buildParameters([text, optionalText], values)).toEqual({
      message: { type: "template", value: "{{ system.project.current.key }}" },
    })
  })
})
