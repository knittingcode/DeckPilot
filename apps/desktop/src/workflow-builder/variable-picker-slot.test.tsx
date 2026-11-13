// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, within } from "@testing-library/react"
import { useState } from "react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { VariableScopeProvider } from "@/components/variable-picker/scope-provider"
import type { ActionType, GraphMapping, GraphNode, ParameterSpec } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { action, agentConnection } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetBackend } from "@/test/tauri"
import { SECRET, fakeVariableAgent } from "@/test/variables"

import { DEFAULT_POLICY } from "./graph"
import { NodeInspector } from "./inspector"
import { MappingEditor } from "./mapping-editor"
import { VARIABLE_PICKER } from "./variable-reference"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

beforeAll(installDomStubs)
beforeEach(() => {
  fakeVariableAgent().install(api)
})
afterEach(() => {
  cleanup()
  resetApi()
  resetBackend()
})

const parameter: ParameterSpec = {
  key: "message",
  label: "Message",
  kind: { type: "text", multiline: false },
  required: true,
}
const logType: ActionType = {
  id: "general.log",
  name: "Log message",
  description: "Writes a message.",
  category: "General",
  requiresProject: false,
  parameters: [parameter],
}
const at = { x: 0, y: 0 }
const live = { agent: agentConnection({ capabilities: ["variables"] }) }

function Harness({
  initial,
  saved,
}: {
  initial: GraphMapping
  saved: (mapping: GraphMapping) => void
}) {
  const [mapping, setMapping] = useState(initial)
  return (
    <VariableScopeProvider context={{ workflow: "w-deploy", projectSource: "execution" }}>
      <MappingEditor
        id="m"
        mapping={mapping}
        parameter={parameter}
        outputs={[]}
        problem={() => null}
        picker={VARIABLE_PICKER}
        onChange={(next) => {
          setMapping(next)
          saved(next)
        }}
      />
    </VariableScopeProvider>
  )
}

describe("the builder's variable picker slot", () => {
  it("is filled, and the inspector's mapping editors show the picker", () => {
    expect(VARIABLE_PICKER).toBeTypeOf("function")
    const node: GraphNode = {
      id: "log",
      type: "action",
      action: "a-log",
      mappings: { message: { type: "variable", reference: "" } },
      policy: DEFAULT_POLICY,
      position: at,
    }
    renderWithApp(
      <VariableScopeProvider context={{ workflow: "w-deploy", projectSource: "execution" }}>
        <NodeInspector
          graph={{
            version: 1,
            nodes: [{ id: "start", type: "trigger", position: at }, node],
            edges: [{ from: "start", port: "out", to: "log" }],
          }}
          node={node}
          issues={[]}
          lookup={{
            actions: [action("a-log", "Log", "general.log")],
            types: [logType],
            workflows: [],
            projects: [],
          }}
          projects={[]}
          loopTargets={[]}
          onChange={() => undefined}
          onDelete={() => undefined}
        />
      </VariableScopeProvider>,
      live,
    )
    expect(screen.getByRole("button", { name: "Choose a variable for Variable" })).toBeVisible()
  })

  it("stores the picked reference of a one-variable mapping, not its value", async () => {
    const saved: GraphMapping[] = []
    const { user } = renderWithApp(
      <Harness initial={{ type: "variable", reference: "" }} saved={(m) => saved.push(m)} />,
      live,
    )
    await user.click(screen.getByRole("button", { name: "Choose a variable for Variable" }))
    await user.click(await screen.findByRole("option", { name: /root folder/i }))
    expect(saved.at(-1)).toEqual({ type: "variable", reference: "shared.paths.root" })
    expect(screen.getByLabelText("Variable")).toHaveValue("shared.paths.root")
    expect(JSON.stringify(saved)).not.toContain("/src")
  })

  it("inserts the picked reference into a text template", async () => {
    const saved: GraphMapping[] = []
    const { user } = renderWithApp(
      <Harness initial={{ type: "template", template: "Root: " }} saved={(m) => saved.push(m)} />,
      live,
    )
    await user.click(screen.getByRole("button", { name: /Variable/ }))
    await user.click(screen.getByRole("button", { name: "Choose a variable for Variable 2" }))
    await user.click(await screen.findByRole("option", { name: /root folder/i }))
    expect(saved.at(-1)).toEqual({ type: "template", template: "Root: {{ shared.paths.root }}" })
  })

  it("never shows a sensitive value", async () => {
    const { user } = renderWithApp(
      <Harness initial={{ type: "variable", reference: "" }} saved={() => undefined} />,
      live,
    )
    await user.click(screen.getByRole("button", { name: "Choose a variable for Variable" }))
    const listbox = await screen.findByRole("listbox")
    const sensitive = within(listbox).getByRole("option", { name: /deploy token/i })
    await user.hover(sensitive)
    expect(document.body.innerHTML).not.toContain(SECRET)
  })

  it("shows nothing while the agent cannot resolve variables", () => {
    for (const agent of [
      agentConnection({ capabilities: [] }),
      agentConnection({ running: false, capabilities: ["variables"] }),
      null,
    ]) {
      renderWithApp(
        <Harness initial={{ type: "variable", reference: "" }} saved={() => undefined} />,
        { agent },
      )
      expect(screen.queryByRole("button", { name: /choose a variable/i })).toBeNull()
      // The reference can still be typed.
      expect(screen.getByLabelText("Variable")).toBeInTheDocument()
      cleanup()
    }
    expect(api.listVariables).not.toHaveBeenCalled()
  })
})
