// Work sessions as the agent would report them, behind the mocked
// `api.getWorkSession` & co. until protocol 8 (W5) is there.

import type {
  Workflow,
  WorkSessionReview,
  WorkSessionSettings,
  WorkSessionSnapshot,
  WorkSessionState,
  WorkSessionSummary,
} from "@/lib/types"

/** 2026-11-12 09:00 UTC. */
export const NOW = Date.UTC(2026, 10, 12, 9, 0, 0)
export const MINUTE = 60_000
export const HOUR = 60 * MINUTE

export function snapshot(
  state: WorkSessionState,
  change: Partial<WorkSessionSnapshot> = {},
): WorkSessionSnapshot {
  const hasSession = state !== "idle"
  return {
    state,
    ...(hasSession ? { session: "session-1", profile: "profile-1" } : {}),
    elapsedMs: 0,
    asOfMs: NOW,
    ...change,
  }
}

export function settings(change: Partial<WorkSessionSettings> = {}): WorkSessionSettings {
  return {
    startWorkflow: null,
    verificationWorkflow: null,
    approvalTtlSeconds: 600,
    reviewGapSeconds: 900,
    elapsedFormat: "minutes",
    ...change,
  }
}

export function review(change: Partial<WorkSessionReview> = {}): WorkSessionReview {
  return { id: 1, kind: "sleep", fromMs: NOW - 2 * HOUR, toMs: NOW - HOUR, ...change }
}

/** A finished session of 2:30 with a flagged one-hour gap inside. */
export function summary(change: Partial<WorkSessionSummary> = {}): WorkSessionSummary {
  return {
    session: "session-0",
    profile: "profile-1",
    state: "stopped",
    requestedAtMs: NOW - 3 * HOUR,
    startedAtMs: NOW - 3 * HOUR,
    stoppedAtMs: NOW - 30 * MINUTE,
    elapsedMs: 2 * HOUR + 30 * MINUTE,
    intervals: [{ seq: 1, startedAtMs: NOW - 3 * HOUR, endedAtMs: NOW - 30 * MINUTE }],
    reviews: [review()],
    transitions: [
      { seq: 1, atMs: NOW - 3 * HOUR, from: "idle", event: "start", to: "starting" },
      { seq: 2, atMs: NOW - 3 * HOUR, from: "starting", event: "startSucceeded", to: "working" },
    ],
    ...change,
  }
}

/** A graph workflow with declared outputs (and inputs). */
export function workflow(id: string, name: string, change: Partial<Workflow> = {}): Workflow {
  return { id, name, description: "", steps: [], ...change }
}
