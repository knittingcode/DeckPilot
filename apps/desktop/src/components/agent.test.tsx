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
import { agentConnection } from "@/test/fixtures"
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
  it.each<[string, AgentCompatibility, RegExp]>([
    [
      "outdated",
      { type: "agentOutdated", ...versions },
      /agent \(0\.4\.0\) is older than this app/,
    ],
    ["newer", { type: "agentNewer", agent: "0.6.0", app: "0.5.0" }, /\(0\.6\.0\) is newer/],
    ["unknown", { type: "unknown", ...versions }, /could not be compared/],
  ])("explains an %s agent and offers a restart", async (_, compatibility, text) => {
    const restarted = agentConnection()
    api.restartAgent.mockResolvedValue(restarted)
    const { user, context } = renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({ compatibility }),
    })
    const alert = screen.getByRole("alert")
    expect(alert).toHaveTextContent("The running agent does not match this app")
    expect(alert).toHaveTextContent(text)
    await user.click(screen.getByRole("button", { name: "Restart agent" }))
    expect(api.restartAgent).toHaveBeenCalled()
    await waitFor(() => {
      expect(context.setAgent).toHaveBeenCalledWith(restarted)
    })
    expect(toast.success).toHaveBeenCalledWith("The agent was restarted")
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
      expect(toast.error).toHaveBeenCalledWith("socket busy")
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
