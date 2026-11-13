// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { WorkSessionState } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { profileView, project } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { emitBackend, resetBackend } from "@/test/tauri"
import { resetToasts, toast } from "@/test/toast"
import {
  HOUR,
  MINUTE,
  NOW,
  review,
  settings,
  snapshot,
  summary,
  workflow,
} from "@/test/work-session"

import { STATE_TEXT, WORK_SESSION_EVENT } from "./model"
import { WorkSessionPage } from "./work-session-page"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

const shop = project("SHOP", { name: "Online Shop" })
const check = workflow("check", "Ready check", {
  outputs: [
    { key: "canFinish", kind: "boolean" },
    { key: "details", kind: "text" },
  ],
})
const textCheck = workflow("text-check", "Text check", {
  outputs: [{ key: "canFinish", kind: "text" }],
})
const open = workflow("open", "Open workspace")

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  resetBackend()
  api.getProfile.mockResolvedValue(
    profileView({ projects: [shop], workflows: [check, textCheck, open] }),
  )
  api.getWorkSessionSettings.mockResolvedValue(settings())
  api.listWorkSessions.mockResolvedValue([])
})
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

async function renderPage(state = snapshot("idle")) {
  api.getWorkSession.mockResolvedValue(state)
  const rendered = renderWithApp(<WorkSessionPage />)
  await screen.findByRole("heading", { name: "Current session" })
  return rendered
}

const SESSION_CONTROLS = /\b(start|verify|check|finish|stop|cancel)\b/i

describe("Work Session page", () => {
  it.each<WorkSessionState>([
    "idle",
    "starting",
    "working",
    "verifyingEnd",
    "readyToFinish",
    "stopping",
  ])("shows the %s state without controls for the session", async (state) => {
    await renderPage(
      snapshot(state, {
        elapsedMs: HOUR + 5 * MINUTE,
        asOfMs: Date.now(),
        project: state === "idle" ? undefined : shop.id,
      }),
    )
    expect(screen.getByText(STATE_TEXT[state].label)).toBeInTheDocument()
    expect(screen.getByText(STATE_TEXT[state].hint)).toBeInTheDocument()
    const timer = screen.queryByRole("timer", { name: "Elapsed" })
    if (state === "idle" || state === "starting") {
      expect(timer).not.toBeInTheDocument()
    } else {
      expect(timer).toHaveTextContent("1:05")
      expect(await screen.findByText("SHOP · Online Shop")).toBeInTheDocument()
    }
    // D11: the session is operated from its key only.
    await screen.findByRole("button", { name: "Save settings" })
    const controls = screen
      .getAllByRole("button")
      .map((button) => button.textContent)
      .filter((name) => SESSION_CONTROLS.test(name))
    expect(controls).toEqual([])
  })

  it("follows the agent's events and ticks once a minute", async () => {
    vi.useFakeTimers({ now: NOW, toFake: ["Date", "setTimeout", "clearTimeout"] })
    api.getWorkSession.mockResolvedValue(snapshot("idle"))
    renderWithApp(<WorkSessionPage />)
    await act(async () => {})
    expect(screen.getByText("No work session")).toBeInTheDocument()

    act(() => {
      emitBackend(
        WORK_SESSION_EVENT,
        snapshot("working", { elapsedMs: 59 * MINUTE + 30_000, asOfMs: NOW }),
      )
    })
    expect(screen.getByText("Working")).toBeInTheDocument()
    expect(screen.getByRole("timer")).toHaveTextContent("0:59")
    act(() => {
      vi.advanceTimersByTime(29_000)
    })
    expect(screen.getByRole("timer")).toHaveTextContent("0:59")
    act(() => {
      vi.advanceTimersByTime(1_000)
    })
    expect(screen.getByRole("timer")).toHaveTextContent("1:00")
    // One timer per minute boundary, re-armed after each redraw.
    for (let minute = 0; minute < 60; minute++) {
      expect(vi.getTimerCount()).toBe(1)
      act(() => {
        vi.advanceTimersByTime(MINUTE)
      })
    }
    expect(screen.getByRole("timer")).toHaveTextContent("2:00")

    // A stopped timer schedules nothing.
    act(() => {
      emitBackend(WORK_SESSION_EVENT, snapshot("stopping", { elapsedMs: 2 * HOUR, asOfMs: NOW }))
    })
    expect(vi.getTimerCount()).toBe(0)
  })

  it("explains an agent without work sessions", async () => {
    api.getWorkSession.mockRejectedValue("not supported by this agent (protocol 7)")
    renderWithApp(<WorkSessionPage />)
    expect(await screen.findByText("Work sessions need a newer agent")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Save settings" })).not.toBeInTheDocument()
  })

  it("says what failed and what to do when the session cannot be loaded", async () => {
    api.getWorkSession.mockRejectedValue("the agent is not running")
    renderWithApp(<WorkSessionPage />)
    expect(
      await screen.findByText(
        "The work session could not be loaded: The agent is not running. Check that the agent is running in Settings › Agent.",
      ),
    ).toBeInTheDocument()
  })

  it("shows notices with what to do and clears them", async () => {
    const failed = snapshot("idle", {
      notices: [{ problem: "startFailed", message: "step Open IDE failed", atMs: NOW }],
    })
    const { user } = await renderPage(failed)
    const notices = screen.getByRole("region", { name: "Notices" })
    expect(within(notices).getByText("The work session did not start")).toBeInTheDocument()
    expect(
      within(notices).getByText(
        "Step Open IDE failed. Check the start workflow's run in its history, then click the key to start again.",
      ),
    ).toBeInTheDocument()

    api.clearWorkSessionNotices.mockRejectedValueOnce("the configuration is locked")
    await user.click(screen.getByRole("button", { name: "Clear notices" }))
    expect(toast.error).toHaveBeenCalledWith("The notices were not cleared", {
      description: "The configuration is locked. Unlock DeckPilot if it is locked, then try again.",
    })

    api.clearWorkSessionNotices.mockResolvedValueOnce(snapshot("idle"))
    await user.click(screen.getByRole("button", { name: "Clear notices" }))
    expect(api.clearWorkSessionNotices).toHaveBeenLastCalledWith(null)
    await waitFor(() => {
      expect(screen.queryByRole("region", { name: "Notices" })).not.toBeInTheDocument()
    })
  })

  it("shows the last end check with its details", async () => {
    await renderPage(
      snapshot("working", {
        verification: {
          id: "v-1",
          outcome: { canFinish: false, details: "2 tests fail" },
          atMs: NOW,
        },
      }),
    )
    expect(screen.getByText("Last end check: Not ready to finish")).toBeInTheDocument()
    expect(screen.getByText("— 2 tests fail")).toBeInTheDocument()
  })
})

describe("Work Session settings", () => {
  it("offers only workflows with a Boolean canFinish for the end check", async () => {
    const { user } = await renderPage()
    await screen.findByRole("button", { name: "Save settings" })
    const unusable = screen.getByRole("list", { name: "Workflows that cannot verify the end" })
    expect(within(unusable).getByText("Text check").parentElement).toHaveTextContent(
      "Its output “canFinish” is not a Boolean",
    )
    expect(within(unusable).getByText("Open workspace").parentElement).toHaveTextContent(
      "Has no output “canFinish”",
    )

    await user.click(screen.getByRole("combobox", { name: "End verification workflow" }))
    expect(screen.getByRole("option", { name: "Text check" })).toHaveAttribute(
      "aria-disabled",
      "true",
    )
    await user.click(screen.getByRole("option", { name: "Ready check" }))

    api.saveWorkSessionSettings.mockImplementation((profile, value) =>
      Promise.resolve({ ...value, profile, updatedAtMs: NOW }),
    )
    await user.click(screen.getByRole("button", { name: "Save settings" }))
    expect(api.saveWorkSessionSettings).toHaveBeenCalledWith(
      "profile-1",
      settings({ verificationWorkflow: "check" }),
    )
    expect(toast.success).toHaveBeenCalledWith("Work session settings saved")
    // The stored answer is the new baseline: nothing left to save.
    expect(screen.getByRole("button", { name: "Save settings" })).toBeDisabled()
  })

  it("leaves gesture timings to the key and Settings › Devices", async () => {
    const { user, context } = await renderPage()
    await screen.findByRole("button", { name: "Save settings" })
    expect(screen.queryByLabelText(/long press/i)).not.toBeInTheDocument()
    expect(screen.queryByLabelText(/double click/i)).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open Settings › Devices" }))
    expect(context.openSettings).toHaveBeenCalledWith("devices")
  })

  it("flags a saved verification workflow that lost its canFinish output", async () => {
    api.getWorkSessionSettings.mockResolvedValue(settings({ verificationWorkflow: "text-check" }))
    const { user } = await renderPage()
    const field = await screen.findByRole("combobox", { name: "End verification workflow" })
    expect(field).toHaveAttribute("aria-invalid", "true")
    expect(field).toHaveAccessibleDescription(
      "Its output “canFinish” is not a Boolean. Change its kind to Boolean.",
    )
    // Even an unrelated change cannot be saved until it is fixed.
    await user.type(screen.getByLabelText("Flag gaps longer than (minutes)"), "0")
    expect(screen.getByRole("button", { name: "Save settings" })).toBeDisabled()
  })

  it("flags numbers out of range", async () => {
    const { user } = await renderPage()
    const approval = await screen.findByLabelText("Approval lasts (minutes)")
    await user.clear(approval)
    await user.type(approval, "0")
    expect(approval).toHaveAccessibleDescription("Enter whole minutes from 1 to 1440.")
    expect(screen.getByRole("button", { name: "Save settings" })).toBeDisabled()
  })

  it("shows only the backend's message of a failure, never other parts of the error", async () => {
    const secret = "s3cr3t-session-token"
    const error = Object.assign(new Error("the verification workflow is not valid"), {
      stack: `Error: at invoke (token=${secret})`,
      cause: { token: secret },
    })
    const { user } = await renderPage()
    const approval = await screen.findByLabelText("Approval lasts (minutes)")
    await user.clear(approval)
    await user.type(approval, "15")
    api.saveWorkSessionSettings.mockRejectedValue(error)
    await user.click(screen.getByRole("button", { name: "Save settings" }))
    expect(toast.error).toHaveBeenCalledWith("The work session settings were not saved", {
      description:
        "The verification workflow is not valid. Unlock DeckPilot if it is locked, then try again.",
    })
    expect(JSON.stringify(toast.error.mock.calls)).not.toContain(secret)
    expect(document.body).not.toHaveTextContent(secret)
  })

  it("says what failed when saving does not work", async () => {
    const { user } = await renderPage()
    const approval = await screen.findByLabelText("Approval lasts (minutes)")
    await user.clear(approval)
    await user.type(approval, "15")
    api.saveWorkSessionSettings.mockRejectedValue("the configuration is locked")
    await user.click(screen.getByRole("button", { name: "Save settings" }))
    expect(toast.error).toHaveBeenCalledWith("The work session settings were not saved", {
      description: "The configuration is locked. Unlock DeckPilot if it is locked, then try again.",
    })
  })

  it("never creates an action instance", async () => {
    const { user } = await renderPage()
    await user.click(await screen.findByRole("button", { name: "Open Layout Designer" }))
    expect(api.createAction).not.toHaveBeenCalled()
    expect(api.duplicateAction).not.toHaveBeenCalled()
    expect(api.updateAction).not.toHaveBeenCalled()
  })
})

describe("Work Session history", () => {
  it("shows sessions with intervals and flagged gaps, and excludes a gap", async () => {
    api.listWorkSessions.mockResolvedValue([summary({ project: shop.id })])
    const { user } = await renderPage()
    const list = await screen.findByRole("list", { name: "Work sessions" })
    expect(within(list).getByLabelText("Duration 2 h 30 min")).toHaveTextContent("2:30")
    expect(within(list).getByText("Finished")).toBeInTheDocument()
    expect(within(list).getByText("1 gap to review")).toBeInTheDocument()
    expect(within(list).getByText("SHOP · Online Shop")).toBeInTheDocument()

    const toggle = within(list).getByRole("button", { expanded: false })
    await user.click(toggle)
    expect(toggle).toHaveAttribute("aria-expanded", "true")
    expect(within(list).getByRole("list", { name: "Intervals" }).children).toHaveLength(1)
    expect(within(list).getByRole("list", { name: "Transitions" }).children).toHaveLength(2)
    const gaps = within(list).getByRole("list", { name: "Flagged gaps" })
    expect(within(gaps).getByText(/The computer was asleep/)).toHaveTextContent("(1:00)")
    expect(within(gaps).getByText("Counted, not reviewed")).toBeInTheDocument()

    api.resolveWorkSessionReview.mockResolvedValue(
      summary({
        project: shop.id,
        elapsedMs: HOUR + 30 * MINUTE,
        intervals: [
          {
            seq: 1,
            startedAtMs: NOW - 3 * HOUR,
            endedAtMs: NOW - 2 * HOUR,
            endReason: "gapExcluded",
          },
          { seq: 2, startedAtMs: NOW - HOUR, endedAtMs: NOW - 30 * MINUTE },
        ],
        reviews: [review({ resolution: "excluded", resolvedAtMs: NOW })],
      }),
    )
    await user.click(within(gaps).getByRole("button", { name: /^Exclude the gap/ }))
    expect(api.resolveWorkSessionReview).toHaveBeenCalledWith("session-0", 1, false)
    expect(await within(list).findByLabelText("Duration 1 h 30 min")).toHaveTextContent("1:30")
    expect(within(list).queryByText("1 gap to review")).not.toBeInTheDocument()
    expect(within(list).getByText("Excluded")).toBeInTheDocument()
    expect(toast.success).toHaveBeenCalledWith("The gap is excluded")

    // An excluded gap can be counted again.
    api.resolveWorkSessionReview.mockResolvedValue(summary({ project: shop.id }))
    await user.click(within(list).getByRole("button", { name: /^Include the gap/ }))
    expect(api.resolveWorkSessionReview).toHaveBeenLastCalledWith("session-0", 1, true)
    expect(await within(list).findByLabelText("Duration 2 h 30 min")).toBeInTheDocument()
  })

  it("says what failed when a gap cannot be excluded", async () => {
    api.listWorkSessions.mockResolvedValue([summary()])
    const { user } = await renderPage()
    const list = await screen.findByRole("list", { name: "Work sessions" })
    await user.click(within(list).getByRole("button", { expanded: false }))
    api.resolveWorkSessionReview.mockRejectedValue("the review was already resolved")
    await user.click(within(list).getByRole("button", { name: /^Exclude the gap/ }))
    expect(toast.error).toHaveBeenCalledWith("The gap was not excluded", {
      description:
        "The review was already resolved. Unlock DeckPilot if it is locked, then try again.",
    })
  })

  it("marks a session that never started and has no time", async () => {
    api.listWorkSessions.mockResolvedValue([
      summary({
        state: "aborted",
        startedAtMs: undefined,
        stoppedAtMs: undefined,
        elapsedMs: 0,
        intervals: [],
        reviews: [],
        notice: { problem: "startFailed", message: "step Open IDE failed", atMs: NOW },
      }),
    ])
    const { user } = await renderPage()
    const list = await screen.findByRole("list", { name: "Work sessions" })
    expect(within(list).getByText("Did not start")).toBeInTheDocument()
    await user.click(within(list).getByRole("button", { expanded: false }))
    expect(within(list).getByText(/the timer never started/)).toBeInTheDocument()
    // Its last notice says why.
    expect(within(list).getByText("The work session did not start")).toBeInTheDocument()
    expect(within(list).getByText(/Step Open IDE failed\./)).toBeInTheDocument()
  })

  it("reloads when the agent's session changes", async () => {
    await renderPage(snapshot("working"))
    await waitFor(() => {
      expect(api.listWorkSessions).toHaveBeenCalledTimes(1)
    })
    act(() => {
      emitBackend(WORK_SESSION_EVENT, snapshot("idle"))
    })
    await waitFor(() => {
      expect(api.listWorkSessions).toHaveBeenCalledTimes(2)
    })
  })

  it("says what failed when the history cannot be loaded", async () => {
    api.listWorkSessions.mockRejectedValue("database is locked")
    await renderPage()
    expect(
      await screen.findByText(
        "The session history could not be loaded: Database is locked. Check that the agent is running in Settings › Agent.",
      ),
    ).toBeInTheDocument()
  })
})
