import { CalendarClock, Plus, X, Zap } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { useTranslation } from "@/i18n"
import type {
  EventField,
  LifecycleEvent,
  Schedule,
  TriggerValue,
  WorkflowInput,
  WorkflowTrigger,
} from "@/lib/types"
import { cn } from "@/lib/utils"

import {
  EVENT_FIELDS,
  FIELD_LABEL,
  LIFECYCLE_LABEL,
  MAX_INTERVAL_MINUTES,
  MAX_TRIGGERS,
  MIN_INTERVAL_MINUTES,
  WEEKDAYS,
  describeTrigger,
  fieldAvailable,
  fieldFits,
} from "./trigger-model"

const SCHEDULE_DEFAULTS: Record<Schedule["every"], Schedule> = {
  interval: { every: "interval", minutes: 15 },
  daily: { every: "daily", at: "09:00" },
  weekly: { every: "weekly", days: ["mon", "tue", "wed", "thu", "fri"], at: "09:00" },
}

/** The value source of an input in a trigger, as a select value. */
function sourceOf(value: TriggerValue | undefined) {
  if (!value) return "default"
  return value.type === "fixed" ? "fixed" : `field:${value.field}`
}

function InputSource({
  trigger,
  input,
  index,
  onChange,
}: {
  trigger: WorkflowTrigger
  input: WorkflowInput
  index: number
  onChange: (value: TriggerValue | undefined) => void
}) {
  const { t } = useTranslation()
  const value = trigger.inputs?.[input.key]
  const fields = EVENT_FIELDS.filter(
    (field) => fieldAvailable(field, trigger) && fieldFits(field, input.kind),
  )
  const label = input.label || input.key
  return (
    <div className="grid gap-1">
      <div className="flex items-center gap-1">
        <span className="flex-1 truncate text-xs">
          {label}
          {input.required && (
            <span className="text-muted-foreground">
              {" "}
              · {t("workflows:triggers.input.required")}
            </span>
          )}
        </span>
        <Select
          value={sourceOf(value)}
          onValueChange={(source) => {
            if (source === "default") onChange(undefined)
            else if (source === "fixed") onChange({ type: "fixed", value: input.default ?? "" })
            else onChange({ type: "event", field: source.slice("field:".length) as EventField })
          }}
        >
          <SelectTrigger
            className="h-7 w-36 text-xs"
            aria-label={t("workflows:triggers.input.source", { index: index + 1, input: label })}
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="default">
              {input.required
                ? t("workflows:triggers.input.noValue")
                : input.default
                  ? t("workflows:triggers.input.defaultValue", { value: input.default })
                  : t("workflows:triggers.input.default")}
            </SelectItem>
            <SelectItem value="fixed">{t("workflows:triggers.input.fixed")}</SelectItem>
            {fields.map((field) => (
              <SelectItem key={field} value={`field:${field}`}>
                {FIELD_LABEL[field]}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      {value?.type === "fixed" && (
        <Input
          aria-label={t("workflows:triggers.input.fixedValue", { index: index + 1, input: label })}
          className="h-7 text-xs"
          value={value.value}
          onChange={(e) => {
            onChange({ type: "fixed", value: e.target.value })
          }}
        />
      )}
      {!value && input.required && (
        <p className="text-xs text-destructive">{t("workflows:triggers.input.needsValue")}</p>
      )}
    </div>
  )
}

function ScheduleFields({
  schedule,
  index,
  onChange,
}: {
  schedule: Schedule
  index: number
  onChange: (schedule: Schedule) => void
}) {
  const { t } = useTranslation()
  return (
    <div className="grid gap-1.5">
      <Select
        value={schedule.every}
        onValueChange={(every) => {
          onChange(SCHEDULE_DEFAULTS[every as Schedule["every"]])
        }}
      >
        <SelectTrigger
          className="h-7 text-xs"
          aria-label={t("workflows:triggers.repeats", { index: index + 1 })}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="interval">{t("workflows:triggers.every.interval")}</SelectItem>
          <SelectItem value="daily">{t("workflows:triggers.every.daily")}</SelectItem>
          <SelectItem value="weekly">{t("workflows:triggers.every.weekly")}</SelectItem>
        </SelectContent>
      </Select>
      {schedule.every === "interval" ? (
        <div className="flex items-center gap-1.5 text-xs">
          <span>{t("workflows:triggers.intervalBefore")}</span>
          <Input
            type="number"
            min={MIN_INTERVAL_MINUTES}
            max={MAX_INTERVAL_MINUTES}
            aria-label={t("workflows:triggers.minutes", { index: index + 1 })}
            className="h-7 w-20 text-xs"
            value={schedule.minutes}
            onChange={(e) => {
              onChange({ every: "interval", minutes: Math.trunc(Number(e.target.value)) })
            }}
          />
          <span>{t("workflows:triggers.intervalAfter", { count: schedule.minutes })}</span>
        </div>
      ) : (
        <>
          {schedule.every === "weekly" && (
            <div
              className="flex flex-wrap gap-1"
              role="group"
              aria-label={t("workflows:triggers.weekdays")}
            >
              {WEEKDAYS.map(({ day, label }) => {
                const on = schedule.days.includes(day)
                return (
                  <Button
                    key={day}
                    variant={on ? "default" : "outline"}
                    size="sm"
                    className="h-6 px-1.5 text-xs"
                    aria-pressed={on}
                    onClick={() => {
                      onChange({
                        ...schedule,
                        days: on
                          ? schedule.days.filter((d) => d !== day)
                          : WEEKDAYS.map((w) => w.day).filter(
                              (d) => d === day || schedule.days.includes(d),
                            ),
                      })
                    }}
                  >
                    {label}
                  </Button>
                )
              })}
            </div>
          )}
          <div className="flex items-center gap-1.5 text-xs">
            <span>{t("workflows:triggers.at")}</span>
            <Input
              type="time"
              aria-label={t("workflows:triggers.time", { index: index + 1 })}
              className="h-7 w-28 text-xs"
              value={schedule.at}
              onChange={(e) => {
                onChange({ ...schedule, at: e.target.value })
              }}
            />
            <span className="text-muted-foreground">{t("workflows:triggers.localTime")}</span>
          </div>
        </>
      )}
    </div>
  )
}

/**
 * The open workflow's triggers: schedules and lifecycle events that start
 * it in the agent, with values for its inputs, and a pause switch.
 */
export function TriggersEditor({
  triggers,
  paused,
  inputs,
  onTriggers,
  onPaused,
}: {
  triggers: WorkflowTrigger[]
  paused: boolean
  inputs: WorkflowInput[]
  onTriggers: (triggers: WorkflowTrigger[]) => void
  onPaused: (paused: boolean) => void
}) {
  const { t } = useTranslation()
  const set = (index: number, trigger: WorkflowTrigger) => {
    onTriggers(triggers.map((other, i) => (i === index ? trigger : other)))
  }
  const full = triggers.length >= MAX_TRIGGERS
  return (
    <section className="grid gap-3 border-t px-4 py-3" aria-label={t("workflows:triggers.title")}>
      <div className="flex items-center gap-2">
        <h3 className="flex-1 text-xs font-medium text-muted-foreground uppercase">
          {t("workflows:triggers.title")}
        </h3>
        {triggers.length > 0 && (
          <div className="flex items-center gap-1.5">
            <Switch
              id="triggers-paused"
              checked={paused}
              onCheckedChange={(checked) => {
                onPaused(checked)
              }}
            />
            <Label htmlFor="triggers-paused" className="text-xs font-normal">
              {t("workflows:triggers.paused")}
            </Label>
          </div>
        )}
      </div>
      <p className="text-xs text-muted-foreground">{t("workflows:triggers.description")}</p>
      {triggers.map((trigger, index) => (
        <div
          key={index}
          role="group"
          aria-label={t("workflows:triggers.group", {
            index: index + 1,
            trigger: describeTrigger(trigger),
          })}
          className={cn("grid gap-2 rounded-md border p-2", paused && "opacity-60")}
        >
          <div className="flex items-center gap-1.5">
            {trigger.type === "schedule" ? (
              <CalendarClock className="size-3.5 text-muted-foreground" aria-hidden />
            ) : (
              <Zap className="size-3.5 text-muted-foreground" aria-hidden />
            )}
            <span className="flex-1 text-xs font-medium">{describeTrigger(trigger)}</span>
            <Button
              variant="ghost"
              size="icon"
              className="size-7"
              aria-label={t("workflows:triggers.remove", { index: index + 1 })}
              onClick={() => {
                onTriggers(triggers.filter((_, i) => i !== index))
              }}
            >
              <X />
            </Button>
          </div>
          {trigger.type === "schedule" ? (
            <ScheduleFields
              schedule={trigger.schedule}
              index={index}
              onChange={(schedule) => {
                set(index, { ...trigger, schedule })
              }}
            />
          ) : (
            <Select
              value={trigger.event}
              onValueChange={(event) => {
                set(index, { ...trigger, event: event as LifecycleEvent })
              }}
            >
              <SelectTrigger
                className="h-7 text-xs"
                aria-label={t("workflows:triggers.eventSelect", { index: index + 1 })}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(Object.keys(LIFECYCLE_LABEL) as LifecycleEvent[]).map((event) => (
                  <SelectItem key={event} value={event}>
                    {LIFECYCLE_LABEL[event]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
          {inputs.map((input) => (
            <InputSource
              key={input.key}
              trigger={trigger}
              input={input}
              index={index}
              onChange={(value) => {
                const next = Object.fromEntries(
                  Object.entries(trigger.inputs ?? {}).filter(([key]) => key !== input.key),
                )
                if (value) next[input.key] = value
                set(index, { ...trigger, inputs: next })
              }}
            />
          ))}
        </div>
      ))}
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          disabled={full}
          onClick={() => {
            onTriggers([...triggers, { type: "schedule", schedule: SCHEDULE_DEFAULTS.interval }])
          }}
        >
          <Plus /> {t("workflows:triggers.addSchedule")}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={full}
          onClick={() => {
            onTriggers([...triggers, { type: "lifecycle", event: "deviceConnected" }])
          }}
        >
          <Plus /> {t("workflows:triggers.addEvent")}
        </Button>
      </div>
    </section>
  )
}
