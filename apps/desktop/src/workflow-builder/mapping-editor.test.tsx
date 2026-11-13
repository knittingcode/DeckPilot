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

  it("adds variables to the text and offers the picker beside them", () => {
    let template = ""
    const picked: string[] = []
    render(
      <PickerHarness
        onTemplate={(t) => (template = t)}
        onPickerShown={(expected) => picked.push(expected ?? "none")}
      />,
    )
    fireEvent.click(screen.getByRole("button", { name: /Variable/ }))
    fireEvent.change(screen.getByLabelText("Variable 1"), {
      target: { value: "shared.paths.root" },
    })
    expect(template).toBe("{{ shared.paths.root }}")
    expect(screen.getByLabelText("Preview").textContent).toBe("Preview: ‹shared.paths.root›")
    // The picker hook inserts a reference.
    fireEvent.click(screen.getByRole("button", { name: "Pick a variable" }))
    expect(template).toBe("{{ system.platform }}")
    expect(picked.length).toBeGreaterThan(0)
  })

  it("edits one variable with its type checked", () => {
    let last: GraphMapping | undefined
    function Single() {
      const [mapping, setMapping] = useState<GraphMapping>({ type: "variable", reference: "" })
      return (
        <MappingEditor
          id="m"
          mapping={mapping}
          parameter={{ key: "n", label: "N", kind: { type: "integer" }, required: false }}
          outputs={outputs}
          problem={() => null}
          onChange={(next) => {
            setMapping(next)
            last = next
          }}
        />
      )
    }
    render(<Single />)
    fireEvent.change(screen.getByLabelText("Variable"), {
      target: { value: "system.project.current.id" },
    })
    expect(last).toEqual({ type: "variable", reference: "system.project.current.id" })
    expect(
      screen.getByText(
        "{{ system.project.current.id }} is a projectId value, the parameter expects a whole number",
      ),
    ).toBeTruthy()
  })
})

function PickerHarness({
  onTemplate,
  onPickerShown,
}: {
  onTemplate: (template: string) => void
  onPickerShown: (expected: string | undefined) => void
}) {
  const [mapping, setMapping] = useState<GraphMapping>({ type: "template", template: "" })
  return (
    <MappingEditor
      id="m"
      mapping={mapping}
      parameter={parameter}
      outputs={outputs}
      problem={() => null}
      picker={({ expected, onPick }) => {
        onPickerShown(expected?.type)
        return (
          <button
            type="button"
            onClick={() => {
              onPick("system.platform")
            }}
          >
            Pick a variable
          </button>
        )
      }}
      onChange={(next) => {
        setMapping(next)
        if (next.type === "template") onTemplate(next.template)
      }}
    />
  )
}
