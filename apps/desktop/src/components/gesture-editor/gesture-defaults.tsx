import { useEffect, useState } from "react"
import { toast } from "sonner"

import { ErrorNotice } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
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
    <Card>
      <CardHeader>
        <CardTitle>Gesture timing</CardTitle>
        <CardDescription>
          Defaults for keys with a double click or long press. A key can use other values in the
          layout designer. Keys that only act on a press are not affected.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form
          className="grid max-w-md gap-4"
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
          {FIELDS.map(({ field, id, hint }) => {
            const { min, max, label } = TIMING_LIMITS[field]
            const problem =
              errors[field] ?? (text[field].trim() === "" && saved ? "Enter a value." : null)
            return (
              <div key={field} className="grid gap-1">
                <Label htmlFor={id}>{label} (ms)</Label>
                <Input
                  id={id}
                  type="number"
                  inputMode="numeric"
                  min={min}
                  max={max}
                  step={10}
                  className="w-32"
                  value={text[field]}
                  disabled={saved === null}
                  aria-invalid={problem ? true : undefined}
                  aria-describedby={`${id}-hint${problem ? ` ${id}-error` : ""}`}
                  onChange={(e) => {
                    setText({ ...text, [field]: e.target.value })
                  }}
                />
                <p id={`${id}-hint`} className="text-xs text-muted-foreground">
                  {hint} {min}–{max} ms, default {DEFAULT_TIMINGS[field]} ms.
                </p>
                {problem && (
                  <p id={`${id}-error`} className="text-xs text-destructive" role="alert">
                    {problem}
                  </p>
                )}
              </div>
            )
          })}
          <div className="flex gap-2">
            <Button type="submit" size="sm" disabled={!valid || !changed || saving}>
              Save
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
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
          </div>
        </form>
      </CardContent>
    </Card>
  )
}
