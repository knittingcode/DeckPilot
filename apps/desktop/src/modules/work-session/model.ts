// The Work Session page's model: elapsed time, settings validation and the
// texts for states, notices and gaps. Pure, so it is tested without React.
// See docs/gestures-and-work-session.md (sections 5 to 10).

import { type TranslationKey, currentLanguage, i18n } from "@/i18n"
import { formatList } from "@/i18n/format"
import { errorCode, rawErrorMessage } from "@/lib/backend-error"
import { sentence } from "@/lib/failure"
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
  if (hours === 0) return i18n.t("common:duration.minutes", { minutes })
  return i18n.t("common:duration.hoursMinutes", {
    hours,
    minutes: String(minutes % 60).padStart(2, "0"),
  })
}

/** Milliseconds until the shown elapsed text changes: the next minute (or second) boundary. */
export function nextTickMs(elapsedMs: number, format: ElapsedFormat): number {
  const unit = format === "seconds" ? 1000 : 60_000
  return unit - (Math.max(0, elapsedMs) % unit)
}

// States -------------------------------------------------------------------------

export const STATE_TEXT: Record<WorkSessionState, { label: TranslationKey; hint: TranslationKey }> =
  {
    idle: {
      label: "workspace:workSession.state.idle.label",
      hint: "workspace:workSession.state.idle.hint",
    },
    starting: {
      label: "workspace:workSession.state.starting.label",
      hint: "workspace:workSession.state.starting.hint",
    },
    working: {
      label: "workspace:workSession.state.working.label",
      hint: "workspace:workSession.state.working.hint",
    },
    verifyingEnd: {
      label: "workspace:workSession.state.verifyingEnd.label",
      hint: "workspace:workSession.state.verifyingEnd.hint",
    },
    readyToFinish: {
      label: "workspace:workSession.state.readyToFinish.label",
      hint: "workspace:workSession.state.readyToFinish.hint",
    },
    stopping: {
      label: "workspace:workSession.state.stopping.label",
      hint: "workspace:workSession.state.stopping.hint",
    },
  }

export const RECORD_TEXT: Record<WorkSessionRecordState, TranslationKey> = {
  starting: "workspace:workSession.record.starting",
  working: "workspace:workSession.record.active",
  verifyingEnd: "workspace:workSession.record.active",
  readyToFinish: "workspace:workSession.record.active",
  stopping: "workspace:workSession.record.stopping",
  stopped: "workspace:workSession.record.stopped",
  aborted: "workspace:workSession.record.aborted",
}

// Notices ------------------------------------------------------------------------

/** What each problem means and what to do about it. */
export const NOTICE_TEXT: Record<
  WorkSessionProblem,
  { title: TranslationKey; next: TranslationKey }
> = {
  startFailed: {
    title: "workspace:workSession.notice.startFailed.title",
    next: "workspace:workSession.notice.startFailed.next",
  },
  startInterrupted: {
    title: "workspace:workSession.notice.startInterrupted.title",
    next: "workspace:workSession.notice.startInterrupted.next",
  },
  verificationFailed: {
    title: "workspace:workSession.notice.verificationFailed.title",
    next: "workspace:workSession.notice.verificationFailed.next",
  },
  verificationInterrupted: {
    title: "workspace:workSession.notice.verificationInterrupted.title",
    next: "workspace:workSession.notice.verificationInterrupted.next",
  },
  notReady: {
    title: "workspace:workSession.notice.notReady.title",
    next: "workspace:workSession.notice.notReady.next",
  },
  approvalExpired: {
    title: "workspace:workSession.notice.approvalExpired.title",
    next: "workspace:workSession.notice.approvalExpired.next",
  },
  approvalInvalidated: {
    title: "workspace:workSession.notice.approvalInvalidated.title",
    next: "workspace:workSession.notice.approvalInvalidated.next",
  },
  needsReview: {
    title: "workspace:workSession.notice.needsReview.title",
    next: "workspace:workSession.notice.needsReview.next",
  },
}

/** A notice as the page shows it: what happened, why, what to do. */
export function noticeText(notice: WorkSessionNotice): { title: string; detail: string } {
  const text = NOTICE_TEXT[notice.problem]
  return {
    title: i18n.t(text.title),
    detail: [sentence(notice.message), sentence(i18n.t(text.next))].filter(Boolean).join(" "),
  }
}

export const REVIEW_KIND_TEXT: Record<WorkSessionReviewKind, TranslationKey> = {
  agentDown: "workspace:workSession.review.agentDown",
  sleep: "workspace:workSession.review.sleep",
  clockBack: "workspace:workSession.review.clockBack",
  clockForward: "workspace:workSession.review.clockForward",
}

/** The result of an end check, as a label. */
export function outcomeKey(outcome: { canFinish: boolean } | undefined): TranslationKey {
  if (outcome === undefined) return "workspace:workSession.check.noResult"
  return outcome.canFinish
    ? "workspace:workSession.check.canFinish"
    : "workspace:workSession.check.notReady"
}

/** Flagged gaps nobody has decided on yet. */
export function openReviews(reviews: WorkSessionReview[]): WorkSessionReview[] {
  return reviews.filter((review) => review.resolution === undefined)
}

/** Whether an error says the agent is older than protocol 8. */
export function isNotSupported(error: unknown): boolean {
  if (errorCode(error) === "invalid.workSessionAgentTooOld") return true
  return /not supported/i.test(rawErrorMessage(error))
}

// Settings ------------------------------------------------------------------------

/** Inputs of the start or verification workflow the agent fills (section 7). */
export const SESSION_INPUTS = ["sessionId", "projectId", "elapsedSeconds"]

export type VerificationIssue = "missingCanFinish" | "canFinishNotBoolean" | "detailsNotText"

export const VERIFICATION_ISSUE_TEXT: Record<VerificationIssue, TranslationKey> = {
  missingCanFinish: "workspace:workSession.verificationIssue.missingCanFinish",
  canFinishNotBoolean: "workspace:workSession.verificationIssue.canFinishNotBoolean",
  detailsNotText: "workspace:workSession.verificationIssue.detailsNotText",
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
  const names = formatList(
    currentLanguage(),
    missing.map((name) => i18n.t("workspace:workSession.issues.quoted", { name })),
  )
  return i18n.t("workspace:workSession.issues.unfilledInputs", { count: missing.length, names })
}

/** Why a workflow cannot be the verification workflow, as text. */
export function verificationWorkflowIssue(workflow: Workflow): string | null {
  const issue = verificationIssue(workflow)
  return issue ? i18n.t(VERIFICATION_ISSUE_TEXT[issue]) : inputIssue(workflow)
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

/**
 * Checks the form against the workflows of the profile. Returns the settings
 * to save, or `null` with the issues per field.
 */
export function validateSettings(
  draft: SettingsDraft,
  workflows: Workflow[],
): { settings: WorkSessionSettings | null; issues: SettingsIssues } {
  const issues: SettingsIssues = {}
  const missingWorkflow = i18n.t("workspace:workSession.issues.missingWorkflow")
  const find = (id: string | null) => (id ? workflows.find((w) => w.id === id) : undefined)

  if (draft.startWorkflow) {
    const workflow = find(draft.startWorkflow)
    const issue = workflow ? inputIssue(workflow) : missingWorkflow
    if (issue) issues.startWorkflow = issue
  }
  if (draft.verificationWorkflow) {
    const workflow = find(draft.verificationWorkflow)
    const issue = workflow ? verificationWorkflowIssue(workflow) : missingWorkflow
    if (issue) issues.verificationWorkflow = issue
  }
  const minutes = i18n.t("workspace:workSession.issues.minutesRange", MINUTES_RANGE)
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
