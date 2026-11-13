// The Work Session page's model: elapsed time, settings validation and the
// texts for states, notices and gaps. Pure, so it is tested without React.
// See docs/gestures-and-work-session.md (sections 5 to 10).

import { errorMessage } from "@/lib/api"
import type {
  ElapsedFormat,
  Workflow,
  WorkSessionNotice,
  WorkSessionProblem,
  WorkSessionRecordState,
  WorkSessionReview,
  WorkSessionReviewKind,
  WorkSessionSettings,
  WorkSessionSnapshot,
  WorkSessionState,
} from "@/lib/types"

import { sentence } from "./failure"

/** Backend event with the latest {@link WorkSessionSnapshot} (protocol 8). */
export const WORK_SESSION_EVENT = "agent-work-session"

/** The settings of a profile that never saved any (schema 9 defaults). */
export const DEFAULT_SETTINGS: WorkSessionSettings = {
  startWorkflow: null,
  verificationWorkflow: null,
  approvalTtlSeconds: 600,
  reviewGapSeconds: 900,
  elapsedFormat: "minutes",
}

/** Approval lifetime and review threshold, in minutes (60–86400 s stored). */
export const MINUTES_RANGE = { min: 1, max: 1440 }

// Elapsed time ---------------------------------------------------------------

/** States in which the timer runs (`WorkSessionState::timer_running`). */
export function timerRunning(state: WorkSessionState): boolean {
  return state === "working" || state === "verifyingEnd" || state === "readyToFinish"
}

/** States in which elapsed time means something (counted or frozen). */
export function showsElapsed(state: WorkSessionState): boolean {
  return timerRunning(state) || state === "stopping"
}

/** Elapsed time of a snapshot at `nowMs`: it keeps growing while the timer runs. */
export function liveElapsedMs(snapshot: WorkSessionSnapshot, nowMs: number): number {
  const since = timerRunning(snapshot.state) ? Math.max(0, nowMs - snapshot.asOfMs) : 0
  return Math.max(0, snapshot.elapsedMs) + since
}

/** `h:mm` (D4), or `h:mm:ss` with the `seconds` format. Hours are not capped at 24. */
export function formatElapsed(ms: number, format: ElapsedFormat = "minutes"): string {
  const total = Math.floor(Math.max(0, ms) / 1000)
  const hours = Math.floor(total / 3600)
  const minutes = String(Math.floor((total % 3600) / 60)).padStart(2, "0")
  if (format === "minutes") return `${hours}:${minutes}`
  return `${hours}:${minutes}:${String(total % 60).padStart(2, "0")}`
}

/** A duration for screen readers and history, e.g. `1 h 05 min`. */
export function describeDuration(ms: number): string {
  const minutes = Math.floor(Math.max(0, ms) / 60_000)
  const hours = Math.floor(minutes / 60)
  if (hours === 0) return `${minutes} min`
  return `${hours} h ${String(minutes % 60).padStart(2, "0")} min`
}

/** Milliseconds until the shown elapsed text changes: the next minute (or second) boundary. */
export function nextTickMs(elapsedMs: number, format: ElapsedFormat): number {
  const unit = format === "seconds" ? 1000 : 60_000
  return unit - (Math.max(0, elapsedMs) % unit)
}

// States -------------------------------------------------------------------------

export const STATE_TEXT: Record<WorkSessionState, { label: string; hint: string }> = {
  idle: {
    label: "No work session",
    hint: "Click the Work Session key to start one.",
  },
  starting: {
    label: "Starting…",
    hint: "The start workflow is running. Time counts once it succeeds. Hold the key to cancel.",
  },
  working: {
    label: "Working",
    hint: "To finish, click the key to run the end check. Without a verification workflow, hold the key instead.",
  },
  verifyingEnd: {
    label: "Checking…",
    hint: "The verification workflow is running; the timer keeps going. Hold the key to cancel the check.",
  },
  readyToFinish: {
    label: "Ready to finish",
    hint: "Click the key to finish and stop the timer. Hold it to keep working.",
  },
  stopping: {
    label: "Stopping…",
    hint: "The session is being closed. The stop time is already fixed.",
  },
}

export const RECORD_TEXT: Record<WorkSessionRecordState, string> = {
  starting: "Starting",
  working: "Active",
  verifyingEnd: "Active",
  readyToFinish: "Active",
  stopping: "Stopping",
  stopped: "Finished",
  aborted: "Did not start",
}

// Notices ------------------------------------------------------------------------

/** What each problem means and what to do about it. */
export const NOTICE_TEXT: Record<WorkSessionProblem, { title: string; next: string }> = {
  startFailed: {
    title: "The work session did not start",
    next: "Check the start workflow's run in its history, then click the key to start again.",
  },
  startInterrupted: {
    title: "Starting the work session was interrupted",
    next: "The start workflow may have run partly. Click the key to start again.",
  },
  verificationFailed: {
    title: "The end check failed",
    next: "The timer keeps running. Check the verification workflow's run, then check again from the key.",
  },
  verificationInterrupted: {
    title: "The end check was interrupted",
    next: "The timer keeps running. Check again from the key.",
  },
  notReady: {
    title: "Not ready to finish yet",
    next: "The verification workflow answered no. The timer keeps running.",
  },
  approvalExpired: {
    title: "The approval to finish expired",
    next: "Run the end check again from the key, then click it to finish.",
  },
  approvalInvalidated: {
    title: "The approval to finish was withdrawn",
    next: "Run the end check again from the key, then click it to finish.",
  },
  needsReview: {
    title: "A gap in the session needs review",
    next: "It is counted. Exclude it in the session history if you did not work then.",
  },
}

/** A notice as the page shows it: what happened, why, what to do. */
export function noticeText(notice: WorkSessionNotice): { title: string; detail: string } {
  const text = NOTICE_TEXT[notice.problem]
  return {
    title: text.title,
    detail: [sentence(notice.message), sentence(text.next)].filter(Boolean).join(" "),
  }
}

export const REVIEW_KIND_TEXT: Record<WorkSessionReviewKind, string> = {
  agentDown: "DeckPilot's agent was not running",
  sleep: "The computer was asleep",
  clockBack: "The clock was set back",
  clockForward: "The clock jumped forward",
}

/** Flagged gaps nobody has decided on yet. */
export function openReviews(reviews: WorkSessionReview[]): WorkSessionReview[] {
  return reviews.filter((review) => review.resolution === undefined)
}

/** Whether an error says the agent is older than protocol 8. */
export function isNotSupported(error: unknown): boolean {
  return /not supported/i.test(errorMessage(error))
}

// Settings ------------------------------------------------------------------------

/** Inputs of the start or verification workflow the agent fills (section 7). */
export const SESSION_INPUTS = ["sessionId", "projectId", "elapsedSeconds"]

export type VerificationIssue = "missingCanFinish" | "canFinishNotBoolean" | "detailsNotText"

export const VERIFICATION_ISSUE_TEXT: Record<VerificationIssue, string> = {
  missingCanFinish: "Has no output “canFinish”. Add a Boolean output named canFinish.",
  canFinishNotBoolean: "Its output “canFinish” is not a Boolean. Change its kind to Boolean.",
  detailsNotText: "Its output “details” is not text. Change its kind to Text.",
}

/** Why a workflow cannot be the verification workflow; like `verification_contract`. */
export function verificationIssue(workflow: Workflow): VerificationIssue | null {
  const outputs = workflow.outputs ?? []
  const canFinish = outputs.find((output) => output.key === "canFinish")
  if (!canFinish) return "missingCanFinish"
  if (canFinish.kind !== "boolean") return "canFinishNotBoolean"
  if (outputs.some((output) => output.key === "details" && output.kind !== "text")) {
    return "detailsNotText"
  }
  return null
}

/** Inputs a run started by a work session could not fill. */
function unfilledInputs(workflow: Workflow): string[] {
  return (workflow.inputs ?? [])
    .filter((input) => input.required && input.default === undefined)
    .filter((input) => !SESSION_INPUTS.includes(input.key))
    .map((input) => input.label ?? input.key)
}

/** Why a workflow cannot run for a work session (start or verification). */
export function inputIssue(workflow: Workflow): string | null {
  const missing = unfilledInputs(workflow)
  if (missing.length === 0) return null
  const names = missing.map((name) => `“${name}”`).join(", ")
  return `Needs the input ${names}, which a work session cannot give. Give it a default or make it optional.`
}

/** Why a workflow cannot be the verification workflow, as text. */
export function verificationWorkflowIssue(workflow: Workflow): string | null {
  const issue = verificationIssue(workflow)
  return issue ? VERIFICATION_ISSUE_TEXT[issue] : inputIssue(workflow)
}

/** The settings form: numbers as typed, so a half-typed value stays visible. */
export interface SettingsDraft {
  startWorkflow: string | null
  verificationWorkflow: string | null
  approvalMinutes: string
  reviewGapMinutes: string
  elapsedFormat: ElapsedFormat
}

export type SettingsField = Exclude<keyof SettingsDraft, "elapsedFormat">
export type SettingsIssues = Partial<Record<SettingsField, string>>

export function draftOf(settings: WorkSessionSettings): SettingsDraft {
  return {
    startWorkflow: settings.startWorkflow,
    verificationWorkflow: settings.verificationWorkflow,
    approvalMinutes: String(Math.round(settings.approvalTtlSeconds / 60)),
    reviewGapMinutes: String(Math.round(settings.reviewGapSeconds / 60)),
    elapsedFormat: settings.elapsedFormat,
  }
}

function integerIn(text: string, range: { min: number; max: number }): number | null {
  if (!/^\d+$/.test(text.trim())) return null
  const value = Number(text)
  return value >= range.min && value <= range.max ? value : null
}

const MISSING_WORKFLOW = "This workflow no longer exists. Choose another one or none."

/**
 * Checks the form against the workflows of the profile. Returns the settings
 * to save, or `null` with the issues per field.
 */
export function validateSettings(
  draft: SettingsDraft,
  workflows: Workflow[],
): { settings: WorkSessionSettings | null; issues: SettingsIssues } {
  const issues: SettingsIssues = {}
  const find = (id: string | null) => (id ? workflows.find((w) => w.id === id) : undefined)

  if (draft.startWorkflow) {
    const workflow = find(draft.startWorkflow)
    const issue = workflow ? inputIssue(workflow) : MISSING_WORKFLOW
    if (issue) issues.startWorkflow = issue
  }
  if (draft.verificationWorkflow) {
    const workflow = find(draft.verificationWorkflow)
    const issue = workflow ? verificationWorkflowIssue(workflow) : MISSING_WORKFLOW
    if (issue) issues.verificationWorkflow = issue
  }
  const minutes = `Enter whole minutes from ${MINUTES_RANGE.min} to ${MINUTES_RANGE.max}.`
  const approval = integerIn(draft.approvalMinutes, MINUTES_RANGE)
  if (approval === null) issues.approvalMinutes = minutes
  const reviewGap = integerIn(draft.reviewGapMinutes, MINUTES_RANGE)
  if (reviewGap === null) issues.reviewGapMinutes = minutes

  if (Object.keys(issues).length > 0 || approval === null || reviewGap === null) {
    return { settings: null, issues }
  }
  return {
    settings: {
      startWorkflow: draft.startWorkflow,
      verificationWorkflow: draft.verificationWorkflow,
      approvalTtlSeconds: approval * 60,
      reviewGapSeconds: reviewGap * 60,
      elapsedFormat: draft.elapsedFormat,
    },
    issues,
  }
}
