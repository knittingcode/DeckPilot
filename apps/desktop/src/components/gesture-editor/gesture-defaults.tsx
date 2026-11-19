import { useEffect, useState } from "react"
import { toast } from "sonner"

import { ErrorNotice } from "@/components/page"
import { Button } from "@/components/ui/button"
import { SettingsGroup, SettingsRow, SettingsSection } from "@/components/settings-layout"
import { Input } from "@/components/ui/input"
import { api, errorMessage } from "@/lib/api"
import type { GestureTimings } from "@/lib/types"

import { DEFAULT_TIMINGS, TIMING_LIMITS, timingError, type TimingField } from "./model"

const FIELDS: { field: TimingField; id: string; hint: string }[] = [
  {
    field: "doubleClickMs",
    id: "gesture-default-double",
    hint: "Time from releasing a key to pressing it again that still counts as a double click. A single click on a key with a double click waits this long.",
  },
  {
    field: "longPressMs",
    id: "gesture-default-long",
    hint: "How long a key must be held to count as a long press.",
  },
]

/**
 * Settings › Devices: the default gesture timings of this computer
 * (`gestures.defaults`). Pages can set other values per key.
 */
export function GestureDefaultsCard() {
  const [saved, setSaved] = useState<GestureTimings | null>(null)
  const [text, setText] = useState<Record<TimingField, string>>({
    doubleClickMs: "",
    longPressMs: "",
  })
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const show = (timings: GestureTimings) => {
    setSaved(timings)
    setText({
      doubleClickMs: String(timings.doubleClickMs),
      longPressMs: String(timings.longPressMs),
    })
  }

  useEffect(() => {
    api.getGestureDefaults().then(show, (e: unknown) => {
      setError(errorMessage(e))
    })
  }, [])

  const errors = {
    doubleClickMs: timingError("doubleClickMs", text.doubleClickMs),
    longPressMs: timingError("longPressMs", text.longPressMs),
  }
  const empty = text.doubleClickMs.trim() === "" || text.longPressMs.trim() === ""
  const valid = !errors.doubleClickMs && !errors.longPressMs && !empty
  const changed =
    saved !== null &&
    (Number(text.doubleClickMs) !== saved.doubleClickMs ||
      Number(text.longPressMs) !== saved.longPressMs)

  const save = (timings: GestureTimings) => {
    setError(null)
    setSaving(true)
    api
      .setGestureDefaults(timings)
      .then(
        (stored) => {
          show(stored)
          toast.success("Gesture timings saved")
        },
        (e: unknown) => {
          setError(errorMessage(e))
        },
      )
      .finally(() => {
        setSaving(false)
      })
  }

  return (
    <SettingsSection
      title="Gesture timing"
      description="Defaults for keys with a double click or long press. A key can use other values in the layout designer. Keys that only act on a press are not affected."
    >
      <form
        className="grid gap-3"
        onSubmit={(e) => {
          e.preventDefault()
          if (valid) {
            save({
              doubleClickMs: Number(text.doubleClickMs),
              longPressMs: Number(text.longPressMs),
            })
          }
        }}
      >
        {error && <ErrorNotice error={error} />}
        <SettingsGroup>
          {FIELDS.map(({ field, id, hint }) => {
            const { min, max, label } = TIMING_LIMITS[field]
            const problem =
              errors[field] ?? (text[field].trim() === "" && saved ? "Enter a value." : null)
            return (
              <SettingsRow
                key={field}
                label={`${label} (ms)`}
                htmlFor={id}
                description={
                  <>
                    <span id={`${id}-hint`}>
                      {hint} {min}–{max} ms, default {DEFAULT_TIMINGS[field]} ms.
                    </span>
                    {problem && (
                      <span id={`${id}-error`} className="block text-destructive" role="alert">
                        {problem}
                      </span>
                    )}
                  </>
                }
              >
                <Input
                  id={id}
                  type="number"
                  inputMode="numeric"
                  min={min}
                  max={max}
                  step={10}
                  className="h-8 w-24 text-right tabular-nums"
                  value={text[field]}
                  disabled={saved === null}
                  aria-invalid={problem ? true : undefined}
                  aria-describedby={`${id}-hint${problem ? ` ${id}-error` : ""}`}
                  onChange={(e) => {
                    setText({ ...text, [field]: e.target.value })
                  }}
                />
              </SettingsRow>
            )
          })}
        </SettingsGroup>
        <div className="flex justify-end gap-2">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={
              saving ||
              saved === null ||
              (saved.doubleClickMs === DEFAULT_TIMINGS.doubleClickMs &&
                saved.longPressMs === DEFAULT_TIMINGS.longPressMs)
            }
            onClick={() => {
              save(DEFAULT_TIMINGS)
            }}
          >
            Restore defaults
          </Button>
          <Button type="submit" size="sm" disabled={!valid || !changed || saving}>
            Save
          </Button>
        </div>
      </form>
    </SettingsSection>
  )
}
