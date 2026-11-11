// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { useState } from "react"
import { afterEach, describe, expect, it } from "vitest"

import type { GraphMapping, ParameterSpec } from "@/lib/types"

import { MappingEditor } from "./mapping-editor"

const parameter: ParameterSpec = {
  key: "message",
  label: "Message",
  kind: { type: "text", multiline: false },
  required: true,
}
const outputs = [
  {
    reference: { node: "wait", output: "waitedMs" },
    label: "Wait · Waited (ms)",
    kind: "integer" as const,
  },
]

function Harness({ onTemplate }: { onTemplate: (template: string) => void }) {
  const [mapping, setMapping] = useState<GraphMapping>({ type: "template", template: "" })
  return (
    <MappingEditor
      id="m"
      mapping={mapping}
      parameter={parameter}
      outputs={outputs}
      problem={(r) => (r.node === "wait" ? null : "missing")}
      values={{ wait: { waitedMs: "4002" } }}
      onChange={(next) => {
        setMapping(next)
        if (next.type === "template") onTemplate(next.template)
      }}
    />
  )
}

describe("mapping editor", () => {
  afterEach(cleanup)

  it("composes text and outputs part by part, with a preview", () => {
    let template = ""
    render(<Harness onTemplate={(t) => (template = t)} />)
    fireEvent.click(screen.getByRole("button", { name: /Text/ }))
    // The new, still empty text part stays editable.
    const text = screen.getByLabelText("Text 1")
    fireEvent.change(text, { target: { value: "Waited " } })
    fireEvent.click(screen.getByRole("button", { name: /Output/ }))
    fireEvent.click(screen.getByRole("button", { name: /Text/ }))
    fireEvent.change(screen.getByLabelText("Text 3"), { target: { value: " ms" } })
    expect(template).toBe("Waited {{ steps.wait.output.waitedMs }} ms")
    expect(screen.getByLabelText("Preview").textContent).toBe("Preview: Waited 4002 ms")
    fireEvent.click(screen.getAllByRole("button", { name: "Remove part" })[0] as HTMLElement)
    expect(template).toBe("{{ steps.wait.output.waitedMs }} ms")
  })
})
