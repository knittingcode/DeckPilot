// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import {
  AgentBadge,
  AgentCompatibilityNotice,
  AgentIndicator,
  AgentStopped,
} from "@/components/agent"
import type { AgentCompatibility } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { agentConnection, agentStatus } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
})
afterEach(cleanup)

const versions = { agent: "0.4.0", app: "0.5.0" }

describe("agent compatibility notice", () => {
  it.each<[string, AgentCompatibility]>([
    ["outdated", { type: "agentOutdated", ...versions }],
    ["newer", { type: "agentNewer", agent: "0.6.0", app: "0.5.0" }],
    ["unknown", { type: "unknown", ...versions }],
  ])("explains an %s agent and restarts it at once when nothing runs", async (_, compatibility) => {
    const restarted = agentConnection()
    api.restartAgent.mockResolvedValue(restarted)
    const { user, context } = renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({ compatibility }),
    })
    const alert = screen.getByRole("alert")
    expect(alert).toHaveTextContent("The running agent does not match this app")
    expect(alert).toHaveTextContent(
      `Agent ${compatibility.type === "compatible" ? "" : compatibility.agent}, app 0.5.0.`,
    )
    expect(alert).toHaveTextContent("Restart the agent so both run the same version.")
    await user.click(screen.getByRole("button", { name: "Restart agent" }))
    expect(api.restartAgent).toHaveBeenCalledWith(false)
    await waitFor(() => {
      expect(context.setAgent).toHaveBeenCalledWith(restarted)
    })
    expect(toast.success).toHaveBeenCalledWith("The agent was restarted")
  })

  it("says when the agent must be replaced and which features are missing", () => {
    renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({
        compatibility: { type: "updateRequired", agent: "0.2.0", app: "0.5.0" },
        capabilities: ["workflows", "notifications"],
      }),
    })
    const alert = screen.getByRole("alert")
    expect(alert).toHaveTextContent("The running agent is too old for this app")
    expect(alert).toHaveTextContent("Nothing can be configured until it is replaced")
    expect(alert).toHaveTextContent(
      "Until then these are unavailable: the encrypted configuration, background workflow runs and their history.",
    )
    expect(alert).not.toHaveTextContent("workflow notifications")
  })

  it("lists no missing features while the capabilities are unknown", () => {
    renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({ compatibility: { type: "agentOutdated", ...versions } }),
    })
    expect(screen.getByRole("alert")).not.toHaveTextContent("unavailable")
  })

  it("asks before interrupting running workflows", async () => {
    api.restartAgent.mockResolvedValue(agentConnection())
    const { user } = renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({
        compatibility: { type: "agentOutdated", ...versions },
        status: agentStatus({ runningExecutions: 2 }),
      }),
    })
    expect(screen.getByRole("alert")).toHaveTextContent(
      "2 workflows are running in the agent. Restarting now interrupts them",
    )
    await user.click(screen.getByRole("button", { name: "Restart agent…" }))
    expect(api.restartAgent).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Wait for them" }))
    expect(screen.queryByRole("button", { name: "Interrupt workflows and restart" })).toBeNull()

    await user.click(screen.getByRole("button", { name: "Restart agent…" }))
    await user.click(screen.getByRole("button", { name: "Interrupt workflows and restart" }))
    expect(api.restartAgent).toHaveBeenCalledWith(true)
  })

  it.each([
    ["compatible", agentConnection()],
    [
      "stopped",
      agentConnection({ running: false, compatibility: { type: "unknown", ...versions } }),
    ],
    ["not yet known", agentConnection({ compatibility: null })],
  ])("stays hidden when the agent is %s", (_, agent) => {
    renderWithApp(<AgentCompatibilityNotice />, { agent })
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
  })

  it("reports a failed restart", async () => {
    api.restartAgent.mockRejectedValue(new Error("socket busy"))
    const { user } = renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({ compatibility: { type: "agentOutdated", ...versions } }),
    })
    await user.click(screen.getByRole("button", { name: "Restart agent" }))
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("The agent was not restarted", {
        description: "Socket busy. Check its log in Settings › Diagnostics.",
      })
    })
    expect(screen.getByRole("button", { name: "Restart agent" })).toBeEnabled()
  })
})

describe("stopped agent", () => {
  it("starts the agent and reports when it does not come up", async () => {
    api.startAgent.mockResolvedValue(agentConnection({ running: false }))
    const { user, context } = renderWithApp(<AgentStopped />, {
      agent: agentConnection({ running: false }),
    })
    await user.click(screen.getByRole("button", { name: "Start agent" }))
    await waitFor(() => {
      expect(context.setAgent).toHaveBeenCalled()
    })
    expect(toast.error).toHaveBeenCalledWith("The agent did not start. Check its log for details.")
  })

  it("explains when the agent cannot be started from the app", () => {
    renderWithApp(<AgentStopped />, { agent: agentConnection({ running: false, canStart: false }) })
    expect(screen.getByRole("button", { name: "Start agent" })).toBeDisabled()
    expect(screen.getByText(/executable was not found/)).toBeInTheDocument()
  })

  it("labels the agent state in badges and the titlebar indicator", async () => {
    const onOpen = vi.fn()
    const { user, rerender, context } = renderWithApp(
      <>
        <AgentBadge />
        <AgentIndicator onOpen={onOpen} />
      </>,
      { agent: null },
    )
    expect(screen.getByText("Checking…")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /Checking the agent/ })).toBeInTheDocument()
    rerender(
      <>
        <AgentBadge />
        <AgentIndicator onOpen={onOpen} />
      </>,
      { ...context, agent: agentConnection({ running: false }) },
    )
    expect(screen.getByText("Stopped")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Agent stopped, open agent settings" }))
    expect(onOpen).toHaveBeenCalled()
  })
})
