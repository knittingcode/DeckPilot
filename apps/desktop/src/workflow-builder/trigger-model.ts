/**
 * Workflow triggers in the editor. The rules mirror the engine's
 * (crates/deckpilot-core/src/trigger.rs); its validation stays the
 * authority and is shown in the workflow panel.
 */
import { i18n } from "@/i18n"
import type {
  EventField,
  ExecutionSummary,
  LifecycleEvent,
  Schedule,
  ValueKind,
  Weekday,
  WorkflowTrigger,
} from "@/lib/types"

import { translatedLabels } from "./variable-reference"

export const MIN_INTERVAL_MINUTES = 1
export const MAX_INTERVAL_MINUTES = 7 * 24 * 60
export const MAX_TRIGGERS = 16

const WEEKDAY_LABEL = translatedLabels<Weekday>({
  mon: "workflows:triggers.weekday.mon",
  tue: "workflows:triggers.weekday.tue",
  wed: "workflows:triggers.weekday.wed",
  thu: "workflows:triggers.weekday.thu",
  fri: "workflows:triggers.weekday.fri",
  sat: "workflows:triggers.weekday.sat",
  sun: "workflows:triggers.weekday.sun",
})

/** The days of the week in order, with short names in the window's language. */
export const WEEKDAYS: readonly { day: Weekday; readonly label: string }[] = (
  ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const
).map((day) => ({
  day,
  get label() {
    return WEEKDAY_LABEL[day]
  },
}))

export const LIFECYCLE_LABEL = translatedLabels<LifecycleEvent>({
  agentStarted: "workflows:triggers.event.agentStarted",
  profileActivated: "workflows:triggers.event.profileActivated",
  projectChanged: "workflows:triggers.event.projectChanged",
  deviceConnected: "workflows:triggers.event.deviceConnected",
  deviceDisconnected: "workflows:triggers.event.deviceDisconnected",
})

export const FIELD_LABEL = translatedLabels<EventField>({
  firedAt: "workflows:triggers.field.firedAt",
  profileName: "workflows:triggers.field.profileName",
  projectKey: "workflows:triggers.field.projectKey",
  projectName: "workflows:triggers.field.projectName",
  previousProjectKey: "workflows:triggers.field.previousProjectKey",
  deviceId: "workflows:triggers.field.deviceId",
  deviceModel: "workflows:triggers.field.deviceModel",
})

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
      return i18n.t("workflows:triggers.schedule.interval", { count: schedule.minutes })
    case "daily":
      return i18n.t("workflows:triggers.schedule.daily", { time: schedule.at })
    case "weekly":
      return i18n.t("workflows:triggers.schedule.weekly", {
        days: dayList(schedule.days) || i18n.t("workflows:triggers.schedule.noDays"),
        time: schedule.at,
      })
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
  return paused ? i18n.t("workflows:triggers.pausedSummary", { triggers: text }) : text
}

/** How a run was started, in words. */
export function triggerText(run: Pick<ExecutionSummary, "trigger" | "triggerDetail">): string {
  const detail = run.triggerDetail
  switch (run.trigger) {
    case "app":
      return i18n.t("workflows:triggers.startedBy.app")
    case "device":
      return i18n.t("workflows:triggers.startedBy.device")
    case "schedule":
      return detail
        ? i18n.t("workflows:triggers.startedBy.scheduleDetail", { detail })
        : i18n.t("workflows:triggers.startedBy.schedule")
    case "lifecycle":
      return detail
        ? i18n.t("workflows:triggers.startedBy.eventDetail", { detail })
        : i18n.t("workflows:triggers.startedBy.event")
    default:
      return i18n.t("workflows:triggers.startedBy.automatic")
  }
}
