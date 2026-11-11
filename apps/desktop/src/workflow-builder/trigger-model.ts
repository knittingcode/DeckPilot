/**
 * Workflow triggers in the editor. The rules mirror the engine's
 * (crates/deckpilot-core/src/trigger.rs); its validation stays the
 * authority and is shown in the workflow panel.
 */
import type {
  EventField,
  ExecutionSummary,
  LifecycleEvent,
  Schedule,
  ValueKind,
  Weekday,
  WorkflowTrigger,
} from "@/lib/types"

export const MIN_INTERVAL_MINUTES = 1
export const MAX_INTERVAL_MINUTES = 7 * 24 * 60
export const MAX_TRIGGERS = 16

export const WEEKDAYS: { day: Weekday; label: string }[] = [
  { day: "mon", label: "Mon" },
  { day: "tue", label: "Tue" },
  { day: "wed", label: "Wed" },
  { day: "thu", label: "Thu" },
  { day: "fri", label: "Fri" },
  { day: "sat", label: "Sat" },
  { day: "sun", label: "Sun" },
]

export const LIFECYCLE_LABEL: Record<LifecycleEvent, string> = {
  agentStarted: "DeckPilot started",
  profileActivated: "Profile activated",
  projectChanged: "Active project changed",
  deviceConnected: "Stream Deck connected",
  deviceDisconnected: "Stream Deck disconnected",
}

export const FIELD_LABEL: Record<EventField, string> = {
  firedAt: "Date and time",
  profileName: "Profile name",
  projectKey: "Project key",
  projectName: "Project name",
  previousProjectKey: "Previous project key",
  deviceId: "Device serial",
  deviceModel: "Device model",
}

export const EVENT_FIELDS = Object.keys(FIELD_LABEL) as EventField[]

/** The kind of value a field holds (`EventField::kind`). */
export function fieldKind(field: EventField): ValueKind {
  return field === "projectKey" || field === "previousProjectKey" ? "projectKey" : "text"
}

/** Whether events of `trigger` carry `field` (`EventField::available`). */
export function fieldAvailable(field: EventField, trigger: WorkflowTrigger): boolean {
  const event = trigger.type === "lifecycle" ? trigger.event : undefined
  switch (field) {
    case "previousProjectKey":
      return event === "projectChanged"
    case "deviceId":
    case "deviceModel":
      return event === "deviceConnected" || event === "deviceDisconnected"
    default:
      return true
  }
}

/** Whether a field fits an input: text inputs take anything. */
export function fieldFits(field: EventField, input: ValueKind) {
  return input === "text" || fieldKind(field) === input
}

function dayList(days: Weekday[]) {
  return WEEKDAYS.filter((w) => days.includes(w.day))
    .map((w) => w.label)
    .join(", ")
}

export function describeSchedule(schedule: Schedule): string {
  switch (schedule.every) {
    case "interval":
      return schedule.minutes === 1 ? "Every minute" : `Every ${schedule.minutes} minutes`
    case "daily":
      return `Daily at ${schedule.at}`
    case "weekly":
      return `${dayList(schedule.days) || "No days"} at ${schedule.at}`
  }
}

/** One line about a trigger. */
export function describeTrigger(trigger: WorkflowTrigger): string {
  return trigger.type === "schedule"
    ? describeSchedule(trigger.schedule)
    : LIFECYCLE_LABEL[trigger.event]
}

/** The start node's summary of a workflow's triggers. */
export function triggersSummary(triggers: WorkflowTrigger[] | undefined, paused?: boolean) {
  if (!triggers?.length) return ""
  const text = triggers.map(describeTrigger).join(" · ")
  return paused ? `Paused · ${text}` : text
}

/** How a run was started, in words. */
export function triggerText(run: Pick<ExecutionSummary, "trigger" | "triggerDetail">): string {
  const detail = run.triggerDetail ? ` (${run.triggerDetail})` : ""
  switch (run.trigger) {
    case "app":
      return "Started in the app"
    case "device":
      return "Started from a Stream Deck"
    case "schedule":
      return `Started by a schedule${detail}`
    case "lifecycle":
      return `Started by an event${detail}`
    default:
      return "Started automatically"
  }
}
