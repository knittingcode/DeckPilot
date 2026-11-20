import { useState } from "react"
import { toast } from "sonner"

import { ErrorNotice, Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { SettingsSection } from "@/components/settings-layout"
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
import { useTranslation } from "@/i18n"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { failureMessage, toastFailure } from "@/lib/failure"
import type { ElapsedFormat, Workflow, WorkSessionSettings } from "@/lib/types"

import {
  MINUTES_RANGE,
  draftOf,
  inputIssue,
  validateSettings,
  verificationWorkflowIssue,
  type SettingsDraft,
  type SettingsIssues,
} from "./model"

const NONE = "none"

type NumberKey = "approvalMinutes" | "reviewGapMinutes"

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
  const { t } = useTranslation()
  return (
    <SettingsSection
      title={t("workspace:workSession.settings.title")}
      description={t("workspace:workSession.settings.description")}
    >
      <div className="grid gap-3">
        {settings.status === "loading" ? (
          <Loading />
        ) : settings.status === "error" ? (
          <ErrorNotice
            error={failureMessage(
              t("workspace:workSession.settings.loadFailed"),
              settings.error,
              t("workspace:workSession.checkAgent"),
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
      </div>
    </SettingsSection>
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
  const { t } = useTranslation()
  const { openSettings } = useAppContext()
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
      aria-label={t("workspace:workSession.settings.form")}
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
            toast.success(t("workspace:workSession.settings.saved"))
          },
          (e: unknown) => {
            setSaving(false)
            toastFailure(
              t("workspace:workSession.settings.saveFailed"),
              e,
              t("workspace:workSession.unlockNext"),
            )
          },
        )
      }}
    >
      <WorkflowField
        id="work-session-start"
        label={t("workspace:workSession.settings.start.label")}
        hint={t("workspace:workSession.settings.start.hint")}
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
        label={t("workspace:workSession.settings.verification.label")}
        hint={t("workspace:workSession.settings.verification.hint")}
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
          <span className="font-medium">{t("workspace:workSession.settings.unusable")}</span>
          <ul aria-label={t("workspace:workSession.settings.unusable")} className="grid gap-1">
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
          label={t("workspace:workSession.settings.approval.label")}
          hint={t("workspace:workSession.settings.approval.hint")}
          field="approvalMinutes"
          draft={draft}
          issues={issues}
          range={MINUTES_RANGE}
          onChange={set}
        />
        <NumberField
          id="work-session-review"
          label={t("workspace:workSession.settings.reviewGap.label")}
          hint={t("workspace:workSession.settings.reviewGap.hint")}
          field="reviewGapMinutes"
          draft={draft}
          issues={issues}
          range={MINUTES_RANGE}
          onChange={set}
        />
      </div>
      <div className="grid gap-2">
        <Label htmlFor="work-session-format">
          {t("workspace:workSession.settings.format.label")}
        </Label>
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
            <SelectItem value="minutes">
              {t("workspace:workSession.settings.format.minutes")}
            </SelectItem>
            <SelectItem value="seconds">
              {t("workspace:workSession.settings.format.seconds")}
            </SelectItem>
          </SelectContent>
        </Select>
      </div>
      <section aria-labelledby="work-session-gestures" className="grid gap-2">
        <h3 id="work-session-gestures" className="text-sm font-medium">
          {t("workspace:workSession.settings.gestures.title")}
        </h3>
        <p className="text-xs text-muted-foreground">
          {t("workspace:workSession.settings.gestures.description")}
        </p>
        <div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              openSettings("devices")
            }}
          >
            {t("workspace:workSession.settings.gestures.open")}
          </Button>
        </div>
      </section>
      <div className="flex gap-2">
        <Button type="submit" disabled={!settings || !dirty || saving}>
          {t("workspace:workSession.settings.save")}
        </Button>
        <Button
          type="button"
          variant="ghost"
          disabled={!dirty || saving}
          onClick={() => {
            setDraft(initial)
          }}
        >
          {t("workspace:workSession.settings.discard")}
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
  const { t } = useTranslation()
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
          <SelectItem value={NONE}>{t("workspace:workSession.settings.none")}</SelectItem>
          {!known && value && (
            <SelectItem value={value}>
              {t("workspace:workSession.settings.deletedWorkflow")}
            </SelectItem>
          )}
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
  onChange,
}: {
  id: string
  label: string
  hint: string
  field: NumberKey
  draft: SettingsDraft
  issues: SettingsIssues
  range: { min: number; max: number }
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
