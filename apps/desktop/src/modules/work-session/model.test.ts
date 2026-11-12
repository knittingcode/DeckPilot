import { describe, expect, it } from "vitest"

import { HOUR, MINUTE, NOW, settings, snapshot, workflow } from "@/test/work-session"

import {
  draftOf,
  formatElapsed,
  inputIssue,
  liveElapsedMs,
  nextTickMs,
  noticeText,
  timerRunning,
  validateSettings,
  verificationIssue,
} from "./model"

describe("elapsed time", () => {
  it("formats h:mm by default (D4)", () => {
    expect(formatElapsed(0)).toBe("0:00")
    expect(formatElapsed(59_999)).toBe("0:00")
    expect(formatElapsed(MINUTE)).toBe("0:01")
    expect(formatElapsed(HOUR + 5 * MINUTE + 30_000)).toBe("1:05")
    expect(formatElapsed(25 * HOUR + 3 * MINUTE)).toBe("25:03")
    expect(formatElapsed(-5000)).toBe("0:00")
  })

  it("formats h:mm:ss with the seconds format", () => {
    expect(formatElapsed(HOUR + 5 * MINUTE + 9_400, "seconds")).toBe("1:05:09")
    expect(formatElapsed(0, "seconds")).toBe("0:00:00")
  })

  it("grows from the snapshot only while the timer runs", () => {
    const working = snapshot("working", { elapsedMs: HOUR, asOfMs: NOW })
    expect(liveElapsedMs(working, NOW + 2 * MINUTE)).toBe(HOUR + 2 * MINUTE)
    expect(liveElapsedMs(working, NOW - MINUTE)).toBe(HOUR)
    const stopping = snapshot("stopping", { elapsedMs: HOUR, asOfMs: NOW })
    expect(liveElapsedMs(stopping, NOW + 2 * MINUTE)).toBe(HOUR)
    expect(
      ["working", "verifyingEnd", "readyToFinish"].every((s) => timerRunning(s as never)),
    ).toBe(true)
    expect(timerRunning("idle") || timerRunning("starting") || timerRunning("stopping")).toBe(false)
  })

  it("ticks at the next minute boundary, or second with h:mm:ss", () => {
    expect(nextTickMs(HOUR + 20_000, "minutes")).toBe(40_000)
    expect(nextTickMs(HOUR, "minutes")).toBe(MINUTE)
    expect(nextTickMs(1_250, "seconds")).toBe(750)
  })
})

describe("verification contract", () => {
  it("needs a Boolean canFinish and text details", () => {
    expect(verificationIssue(workflow("w", "W"))).toBe("missingCanFinish")
    expect(
      verificationIssue(workflow("w", "W", { outputs: [{ key: "canFinish", kind: "text" }] })),
    ).toBe("canFinishNotBoolean")
    expect(
      verificationIssue(
        workflow("w", "W", {
          outputs: [
            { key: "canFinish", kind: "boolean" },
            { key: "details", kind: "integer" },
          ],
        }),
      ),
    ).toBe("detailsNotText")
    expect(
      verificationIssue(
        workflow("w", "W", {
          outputs: [
            { key: "canFinish", kind: "boolean" },
            { key: "details", kind: "text" },
          ],
        }),
      ),
    ).toBeNull()
  })

  it("accepts only inputs a session can fill", () => {
    expect(
      inputIssue(
        workflow("w", "W", {
          inputs: [
            { key: "sessionId", kind: "text", required: true },
            { key: "ticket", kind: "text", required: true, default: "X-1" },
            { key: "note", kind: "text" },
          ],
        }),
      ),
    ).toBeNull()
    expect(
      inputIssue(
        workflow("w", "W", {
          inputs: [{ key: "ticket", label: "Ticket", kind: "text", required: true }],
        }),
      ),
    ).toContain("“Ticket”")
  })
})

describe("settings validation", () => {
  const check = workflow("check", "Check", { outputs: [{ key: "canFinish", kind: "boolean" }] })
  const plain = workflow("plain", "Plain")

  it("turns a valid form into settings", () => {
    const draft = { ...draftOf(settings()), verificationWorkflow: "check", approvalMinutes: "15" }
    expect(validateSettings(draft, [check, plain])).toEqual({
      settings: settings({ verificationWorkflow: "check", approvalTtlSeconds: 900 }),
      issues: {},
    })
  })

  it("saves only what the form edits, not the answer's profile and time", () => {
    const saved = settings({ profile: "profile-1", updatedAtMs: NOW, reviewGapSeconds: 1800 })
    expect(validateSettings(draftOf(saved), [check]).settings).toEqual(
      settings({ reviewGapSeconds: 1800 }),
    )
  })

  it("flags a verification workflow without a Boolean canFinish output", () => {
    const draft = { ...draftOf(settings()), verificationWorkflow: "plain" }
    const { settings: result, issues } = validateSettings(draft, [check, plain])
    expect(result).toBeNull()
    expect(issues.verificationWorkflow).toMatch(/canFinish/)
  })

  it("flags deleted workflows and out-of-range numbers", () => {
    const draft = {
      ...draftOf(settings()),
      startWorkflow: "gone",
      approvalMinutes: "0",
      reviewGapMinutes: "abc",
    }
    const { issues } = validateSettings(draft, [check])
    expect(Object.keys(issues).sort()).toEqual([
      "approvalMinutes",
      "reviewGapMinutes",
      "startWorkflow",
    ])
  })
})

it("says what a notice means and what to do", () => {
  expect(
    noticeText({ problem: "startFailed", message: "the step Open IDE failed", atMs: NOW }),
  ).toEqual({
    title: "The work session did not start",
    detail:
      "The step Open IDE failed. Check the start workflow's run in its history, then click the key to start again.",
  })
})
