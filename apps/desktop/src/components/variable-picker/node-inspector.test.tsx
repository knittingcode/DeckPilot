// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { ParameterField } from "@/components/parameter-field"
import type { ActionType, GraphNode, ParameterSpec, ParameterValue } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { action, agentConnection } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetBackend } from "@/test/tauri"
import { fakeVariableAgent } from "@/test/variables"
import { DEFAULT_POLICY } from "@/workflow-builder/graph"
import { NodeInspector } from "@/workflow-builder/inspector"

import { VariableScopeProvider } from "./scope-provider"

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

const logType: ActionType = {
  id: "general.log",
  name: "Log message",
  description: "Writes a message.",
  category: "General",
  requiresProject: false,
  parameters: [
    { key: "message", label: "Message", kind: { type: "text", multiline: false }, required: true },
  ],
}

const at = { x: 0, y: 0 }

describe("the workflow builder's node inspector", () => {
  it("uses the same picker, resolved in the workflow's run context", async () => {
    const node: GraphNode = {
      id: "log",
      type: "action",
      action: "a-log",
      overrides: { message: { type: "template", value: "{{ system.project.current.key }}" } },
      policy: DEFAULT_POLICY,
      position: at,
    }
    const changes: GraphNode[] = []
    const { user } = renderWithApp(
      // As the builder wraps its inspector.
      <VariableScopeProvider
        context={{ workflow: "w-deploy", projectSource: "execution" }}
        names={{ workflow: "Deploy" }}
      >
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
          onChange={(change) => {
            changes.push(change(node))
          }}
          onDelete={() => undefined}
        />
      </VariableScopeProvider>,
      { agent: agentConnection({ capabilities: ["variables"] }) },
    )
    const references = screen.getByRole("list", { name: "References" })
    await waitFor(() => {
      expect(references).toHaveTextContent("Available when it runs")
    })
    expect(references).toHaveTextContent("the project of the run (known when it runs)")
    expect(references).toHaveTextContent("workflow Deploy")

    await user.click(screen.getByRole("button", { name: /insert a variable into value/i }))
    await user.click(await screen.findByRole("option", { name: /root folder/i }))
    const last = changes.at(-1)
    const saved: ParameterValue | undefined =
      last?.type === "action" ? last.overrides?.message : undefined
    expect(saved?.type).toBe("template")
    expect(saved?.value).toContain("{{ shared.paths.root }}")
    expect(api.listVariables).toHaveBeenCalledWith(
      expect.objectContaining({ workflow: "w-deploy", projectSource: "execution" }),
      expect.anything(),
    )
  })

  it("is off where nothing resolves references, such as module settings", () => {
    renderWithApp(
      <VariableScopeProvider disabled>
        <ParameterField
          spec={logType.parameters[0] as ParameterSpec}
          value={{ type: "text", value: "" }}
          onChange={() => undefined}
        />
      </VariableScopeProvider>,
      { agent: agentConnection({ capabilities: ["variables"] }) },
    )
    expect(screen.queryByRole("button", { name: /insert a variable/i })).toBeNull()
  })
})
