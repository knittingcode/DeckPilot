// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { useState } from "react"
import { afterEach, describe, expect, it } from "vitest"

import type { GraphNode, WorkflowInput, WorkflowTrigger } from "@/lib/types"

import { describeNode } from "./describe"
import {
  describeTrigger,
  fieldAvailable,
  fieldFits,
  triggerText,
  triggersSummary,
} from "./trigger-model"
import { TriggersEditor } from "./triggers-editor"

const inputs: WorkflowInput[] = [
  { key: "serial", label: "Serial", kind: "text", required: true },
  { key: "count", kind: "integer", default: "2" },
]

function Harness({
  initial,
  onChange,
}: {
  initial: WorkflowTrigger[]
  onChange: (triggers: WorkflowTrigger[], paused: boolean) => void
}) {
  const [triggers, setTriggers] = useState(initial)
  const [paused, setPaused] = useState(false)
  return (
    <TriggersEditor
      triggers={triggers}
      paused={paused}
      inputs={inputs}
      onTriggers={(next) => {
        setTriggers(next)
        onChange(next, paused)
      }}
      onPaused={(next) => {
        setPaused(next)
        onChange(triggers, next)
      }}
    />
  )
}

describe("triggers editor", () => {
  afterEach(cleanup)

  it("adds schedules and events, edits them and pauses them", () => {
    let triggers: WorkflowTrigger[] = []
    let paused = false
    render(
      <Harness
        initial={[]}
        onChange={(t, p) => {
          triggers = t
          paused = p
        }}
      />,
    )
    fireEvent.click(screen.getByRole("button", { name: /Schedule/ }))
    expect(triggers).toEqual([{ type: "schedule", schedule: { every: "interval", minutes: 15 } }])
    fireEvent.change(screen.getByLabelText("Trigger 1 minutes"), { target: { value: "5" } })
    expect(screen.getByRole("group", { name: "Trigger 1: Every 5 minutes" })).toBeTruthy()
    // The required input says it needs a value.
    expect(screen.getByText("Needs a value.")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: /Event/ }))
    expect(triggers[1]).toEqual({ type: "lifecycle", event: "deviceConnected" })
    expect(screen.getByRole("group", { name: "Trigger 2: Stream Deck connected" })).toBeTruthy()

    fireEvent.click(screen.getByRole("switch"))
    expect(paused).toBe(true)
    fireEvent.click(screen.getByRole("button", { name: "Remove trigger 1" }))
    expect(triggers).toHaveLength(1)
  })

  it("edits weekdays of a weekly schedule in week order", () => {
    let triggers: WorkflowTrigger[] = []
    render(
      <Harness
        initial={[{ type: "schedule", schedule: { every: "weekly", days: ["wed"], at: "08:30" } }]}
        onChange={(t) => (triggers = t)}
      />,
    )
    fireEvent.click(screen.getByRole("button", { name: "Mon" }))
    expect(triggers[0]).toMatchObject({ schedule: { days: ["mon", "wed"] } })
    fireEvent.change(screen.getByLabelText("Trigger 1 time"), { target: { value: "07:15" } })
    expect(describeTrigger(triggers[0] as WorkflowTrigger)).toBe("Mon, Wed at 07:15")
  })
})

describe("trigger model", () => {
  const connected: WorkflowTrigger = { type: "lifecycle", event: "deviceConnected" }
  const daily: WorkflowTrigger = { type: "schedule", schedule: { every: "daily", at: "09:00" } }

  it("offers only the event fields a trigger has, of a fitting kind", () => {
    expect(fieldAvailable("deviceId", connected)).toBe(true)
    expect(fieldAvailable("deviceId", daily)).toBe(false)
    expect(fieldAvailable("previousProjectKey", connected)).toBe(false)
    expect(fieldAvailable("firedAt", daily)).toBe(true)
    expect(fieldFits("projectKey", "text")).toBe(true)
    expect(fieldFits("deviceId", "integer")).toBe(false)
    expect(fieldFits("projectKey", "projectKey")).toBe(true)
  })

  it("summarizes triggers on the start node and in run history", () => {
    expect(triggersSummary([daily, connected])).toBe("Daily at 09:00 · Stream Deck connected")
    expect(triggersSummary([daily], true)).toBe("Paused · Daily at 09:00")
    const start: GraphNode = { id: "start", type: "trigger", position: { x: 0, y: 0 } }
    const lookup = { actions: [], types: [], workflows: [], projects: [], triggers: [daily] }
    expect(describeNode(start, lookup)).toEqual({ title: "Start", detail: "Daily at 09:00" })
    expect(triggerText({ trigger: "schedule", triggerDetail: "every 15 min" })).toBe(
      "Started by a schedule (every 15 min)",
    )
    expect(triggerText({ trigger: "other" })).toBe("Started automatically")
  })
})
