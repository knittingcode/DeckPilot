import { useState } from "react"
import { toast } from "sonner"

import { ErrorNotice, Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { AsyncState } from "@/hooks/use-async"
import { api } from "@/lib/api"
import type { ElapsedFormat, Workflow, WorkSessionSettings } from "@/lib/types"

import { failureMessage, toastFailure } from "./failure"
import {
  DOUBLE_CLICK_MS,
  LONG_PRESS_MS,
  MINUTES_RANGE,
  draftOf,
  inputIssue,
  validateSettings,
  verificationWorkflowIssue,
  type SettingsDraft,
  type SettingsIssues,
} from "./model"

const NONE = "none"

type NumberKey = "approvalMinutes" | "reviewGapMinutes" | "doubleClickMs" | "longPressMs"

/** The work session settings of the profile being edited. */
export function SettingsCard({
  profileId,
  settings,
  workflows,
  onSaved,
}: {
  profileId: string
  settings: AsyncState<WorkSessionSettings>
  workflows: Workflow[]
  onSaved: (settings: WorkSessionSettings) => void
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Settings</CardTitle>
        <CardDescription>
          For sessions started while this profile is active. Saving withdraws an approval to finish,
          so a session that is ready to finish needs a new end check.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {settings.status === "loading" ? (
          <Loading />
        ) : settings.status === "error" ? (
          <ErrorNotice
            error={failureMessage(
              "The work session settings could not be loaded",
              settings.error,
              "Check that the agent is running in Settings › Agent.",
            )}
          />
        ) : (
          <SettingsForm
            key={profileId}
            profileId={profileId}
            saved={settings.data}
            workflows={workflows}
            onSaved={onSaved}
          />
        )}
      </CardContent>
    </Card>
  )
}

function SettingsForm({
  profileId,
  saved,
  workflows,
  onSaved,
}: {
  profileId: string
  saved: WorkSessionSettings
  workflows: Workflow[]
  onSaved: (settings: WorkSessionSettings) => void
}) {
  const [initial, setInitial] = useState(() => draftOf(saved))
  const [draft, setDraft] = useState(initial)
  const [saving, setSaving] = useState(false)
  const { settings, issues } = validateSettings(draft, workflows)
  const dirty = JSON.stringify(draft) !== JSON.stringify(initial)
  const set = <K extends keyof SettingsDraft>(key: K, value: SettingsDraft[K]) => {
    setDraft((current) => ({ ...current, [key]: value }))
  }
  const unusable = workflows
    .map((workflow) => ({ workflow, issue: verificationWorkflowIssue(workflow) }))
    .filter((entry): entry is { workflow: Workflow; issue: string } => entry.issue !== null)

  return (
    <form
      className="grid max-w-xl gap-5"
      aria-label="Work session settings"
      onSubmit={(event) => {
        event.preventDefault()
        if (!settings) return
        setSaving(true)
        api.saveWorkSessionSettings(profileId, settings).then(
          (stored) => {
            const next = draftOf(stored)
            setInitial(next)
            setDraft(next)
            setSaving(false)
            onSaved(stored)
            toast.success("Work session settings saved")
          },
          (e: unknown) => {
            setSaving(false)
            toastFailure(
              "The work session settings were not saved",
              e,
              "Unlock DeckPilot if it is locked, then try again.",
            )
          },
        )
      }}
    >
      <WorkflowField
        id="work-session-start"
        label="Start workflow"
        hint="Runs when the key starts a session; time counts once it succeeds. If it fails, the session does not start."
        value={draft.startWorkflow}
        workflows={workflows}
        disabledIssue={inputIssue}
        issue={issues.startWorkflow}
        onChange={(value) => {
          set("startWorkflow", value)
        }}
      />
      <WorkflowField
        id="work-session-verification"
        label="End verification workflow"
        hint="Must output canFinish (Boolean): true allows finishing, false keeps the session running. An optional text output details is shown with the result. Without one, holding the key in a running session makes it ready to finish at once."
        value={draft.verificationWorkflow}
        workflows={workflows}
        disabledIssue={verificationWorkflowIssue}
        issue={issues.verificationWorkflow}
        onChange={(value) => {
          set("verificationWorkflow", value)
        }}
      />
      {unusable.length > 0 && (
        <div className="grid gap-1 text-sm">
          <span className="font-medium">Workflows that cannot verify the end</span>
          <ul aria-label="Workflows that cannot verify the end" className="grid gap-1">
            {unusable.map(({ workflow, issue }) => (
              <li key={workflow.id} className="text-muted-foreground">
                <span className="text-foreground">{workflow.name}</span>: {issue}
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="grid gap-4 sm:grid-cols-2">
        <NumberField
          id="work-session-approval"
          label="Approval lasts (minutes)"
          hint="How long a passed end check allows finishing."
          field="approvalMinutes"
          draft={draft}
          issues={issues}
          range={MINUTES_RANGE}
          onChange={set}
        />
        <NumberField
          id="work-session-review"
          label="Flag gaps longer than (minutes)"
          hint="Longer gaps (sleep, agent not running) are counted and flagged in the history."
          field="reviewGapMinutes"
          draft={draft}
          issues={issues}
          range={MINUTES_RANGE}
          onChange={set}
        />
      </div>
      <div className="grid gap-2">
        <Label htmlFor="work-session-format">Elapsed time on keys</Label>
        <Select
          value={draft.elapsedFormat}
          onValueChange={(value) => {
            set("elapsedFormat", value as ElapsedFormat)
          }}
        >
          <SelectTrigger id="work-session-format" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="minutes">h:mm (redrawn once a minute)</SelectItem>
            <SelectItem value="seconds">h:mm:ss (redrawn every second)</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <fieldset className="grid gap-3">
        <legend className="mb-1 text-sm font-medium">Gestures of work session keys</legend>
        <p className="text-xs text-muted-foreground">
          Click starts, checks and finishes; long press cancels, keeps working, or finishes without
          a verification workflow. Leave empty to use the defaults ({LONG_PRESS_MS.default} ms long
          press, {DOUBLE_CLICK_MS.default} ms double click).
        </p>
        <div className="grid gap-4 sm:grid-cols-2">
          <NumberField
            id="work-session-long-press"
            label="Long press (ms)"
            hint={`${LONG_PRESS_MS.min}–${LONG_PRESS_MS.max} ms of holding.`}
            field="longPressMs"
            draft={draft}
            issues={issues}
            range={LONG_PRESS_MS}
            placeholder={String(LONG_PRESS_MS.default)}
            onChange={set}
          />
          <NumberField
            id="work-session-double-click"
            label="Double click (ms)"
            hint={`${DOUBLE_CLICK_MS.min}–${DOUBLE_CLICK_MS.max} ms between clicks.`}
            field="doubleClickMs"
            draft={draft}
            issues={issues}
            range={DOUBLE_CLICK_MS}
            placeholder={String(DOUBLE_CLICK_MS.default)}
            onChange={set}
          />
        </div>
      </fieldset>
      <div className="flex gap-2">
        <Button type="submit" disabled={!settings || !dirty || saving}>
          Save settings
        </Button>
        <Button
          type="button"
          variant="ghost"
          disabled={!dirty || saving}
          onClick={() => {
            setDraft(initial)
          }}
        >
          Discard changes
        </Button>
      </div>
    </form>
  )
}

function WorkflowField({
  id,
  label,
  hint,
  value,
  workflows,
  disabledIssue,
  issue,
  onChange,
}: {
  id: string
  label: string
  hint: string
  value: string | null
  workflows: Workflow[]
  disabledIssue: (workflow: Workflow) => string | null
  issue?: string
  onChange: (value: string | null) => void
}) {
  const known = value === null || workflows.some((w) => w.id === value)
  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>{label}</Label>
      <Select
        value={value ?? NONE}
        onValueChange={(next) => {
          onChange(next === NONE ? null : next)
        }}
      >
        <SelectTrigger
          id={id}
          className="w-full"
          aria-invalid={issue ? true : undefined}
          aria-describedby={`${id}-hint`}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={NONE}>None</SelectItem>
          {!known && value && <SelectItem value={value}>Deleted workflow</SelectItem>}
          {workflows.map((workflow) => (
            <SelectItem
              key={workflow.id}
              value={workflow.id}
              disabled={workflow.id !== value && disabledIssue(workflow) !== null}
            >
              {workflow.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <p
        id={`${id}-hint`}
        className={issue ? "text-xs text-destructive" : "text-xs text-muted-foreground"}
      >
        {issue ?? hint}
      </p>
    </div>
  )
}

function NumberField({
  id,
  label,
  hint,
  field,
  draft,
  issues,
  range,
  placeholder,
  onChange,
}: {
  id: string
  label: string
  hint: string
  field: NumberKey
  draft: SettingsDraft
  issues: SettingsIssues
  range: { min: number; max: number }
  placeholder?: string
  onChange: (field: NumberKey, value: string) => void
}) {
  const issue = issues[field]
  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        inputMode="numeric"
        min={range.min}
        max={range.max}
        placeholder={placeholder}
        value={draft[field]}
        aria-invalid={issue ? true : undefined}
        aria-describedby={`${id}-hint`}
        onChange={(event) => {
          onChange(field, event.target.value)
        }}
      />
      <p
        id={`${id}-hint`}
        className={issue ? "text-xs text-destructive" : "text-xs text-muted-foreground"}
      >
        {issue ?? hint}
      </p>
    </div>
  )
}
