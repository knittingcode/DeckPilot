import { ExternalLink, Trash2 } from "lucide-react"
import { useState } from "react"

import { ModalShell } from "@/components/modal-shell"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { usageKey, usageWorkflow } from "@/lib/action-references"
import { api, errorMessage } from "@/lib/api"
import { describeUsage } from "@/lib/format"
import type {
  ActionInstance,
  ActionReferenceChange,
  ReferenceChange,
  Usage,
  Workflow,
} from "@/lib/types"

/** How a workflow usage is started besides being run by hand. */
function triggerNote(workflow: Workflow | undefined): string | null {
  const count = workflow?.triggers?.length ?? 0
  if (!workflow || count === 0) return null
  const what = count === 1 ? "1 trigger" : `${count} triggers`
  return workflow.triggersPaused ? `${what}, paused` : `started by ${what}`
}

/** Every place that uses an action, each with a way to open it. */
export function ReferenceList({
  usages,
  workflows,
  onOpen,
}: {
  usages: Usage[]
  workflows: Workflow[]
  onOpen?: (usage: Usage) => void
}) {
  if (usages.length === 0) {
    return <p className="text-sm text-muted-foreground">Not used in any workflow or layout.</p>
  }
  return (
    <ul className="grid gap-1 text-sm" aria-label="References">
      {usages.map((usage) => {
        const note = triggerNote(usageWorkflow(workflows, usage))
        return (
          <li key={usageKey(usage)} className="flex items-center justify-between gap-2">
            <span className="min-w-0 text-muted-foreground">
              {describeUsage(usage)}
              {note && <span className="block text-xs">Workflow {note}</span>}
            </span>
            {onOpen && (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                aria-label={`Open ${describeUsage(usage)}`}
                onClick={() => {
                  onOpen(usage)
                }}
              >
                <ExternalLink /> Open
              </Button>
            )}
          </li>
        )
      })}
    </ul>
  )
}

const UNBIND = "unbind"
const REPLACE = "replace:"

function changeOf(choice: string | undefined): ReferenceChange | null {
  if (choice === UNBIND) return { type: "unbind" }
  if (choice?.startsWith(REPLACE)) return { type: "replace", action: choice.slice(REPLACE.length) }
  return null
}

function ChoiceSelect({
  value,
  candidates,
  label,
  placeholder,
  onChange,
}: {
  value: string
  candidates: ActionInstance[]
  label: string
  placeholder: string
  onChange: (value: string) => void
}) {
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger className="w-full sm:w-64" aria-label={label}>
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        {candidates.map((a) => (
          <SelectItem key={a.id} value={`${REPLACE}${a.id}`}>
            Replace with “{a.name}”
          </SelectItem>
        ))}
        <SelectItem value={UNBIND}>Unbind</SelectItem>
      </SelectContent>
    </Select>
  )
}

/**
 * Deletes an action. While it is referenced, every reference needs a
 * decision first: replace it with another action or unbind it. Nothing is
 * changed until the user confirms, and no reference is removed without a
 * decision for it.
 */
export function DeleteActionDialog({
  open,
  profileId,
  action,
  usages,
  candidates,
  onClose,
  onDeleted,
}: {
  open: boolean
  profileId: string
  action: ActionInstance
  usages: Usage[]
  /** The actions a reference can be pointed at instead. */
  candidates: ActionInstance[]
  onClose: () => void
  onDeleted: () => void
}) {
  const [choices, setChoices] = useState<Record<string, string>>({})
  const [all, setAll] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const others = candidates.filter((a) => a.id !== action.id)
  const changes: ActionReferenceChange[] = usages.flatMap((usage) => {
    const change = changeOf(choices[usageKey(usage)])
    return change ? [{ usage, change }] : []
  })
  const undecided = usages.length - changes.length

  const confirm = async () => {
    setBusy(true)
    setError(null)
    try {
      if (changes.length > 0) {
        const remaining = await api.changeActionReferences(profileId, action.id, changes)
        if (remaining.length > 0) {
          setError(`Still used in ${remaining.map(describeUsage).join("; ")}. Nothing was deleted.`)
          return
        }
      }
      await api.deleteAction(profileId, action.id)
      onDeleted()
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <ModalShell
      presentation="dialog"
      open={open}
      size="medium"
      icon={Trash2}
      title={`Delete “${action.name}”?`}
      description={
        usages.length === 0
          ? "The action is removed from this profile. This cannot be undone."
          : `The action is used in ${usages.length === 1 ? "1 place" : `${usages.length} places`}. Choose for each whether it uses another action instead or is unbound; then the action is deleted. This cannot be undone.`
      }
      error={error}
      onClose={onClose}
      secondary={{ label: "Cancel", onClick: onClose }}
      primary={{
        label: usages.length === 0 ? "Delete" : "Apply and delete",
        loadingLabel: "Deleting…",
        variant: "destructive",
        loading: busy,
        disabled: undecided > 0,
        onClick: () => void confirm(),
      }}
    >
      {usages.length > 0 && (
        <div className="grid gap-4">
          <div className="grid gap-2">
            <p className="text-sm font-medium">All references</p>
            <ChoiceSelect
              value={all}
              candidates={others}
              label="For all references"
              placeholder="Choose for all"
              onChange={(value) => {
                setAll(value)
                setChoices(Object.fromEntries(usages.map((u) => [usageKey(u), value])))
              }}
            />
          </div>
          <ul className="grid gap-3" aria-label="References to change">
            {usages.map((usage) => {
              const key = usageKey(usage)
              return (
                <li key={key} className="grid gap-2 sm:flex sm:items-center sm:justify-between">
                  <span className="text-sm">{describeUsage(usage)}</span>
                  <ChoiceSelect
                    value={choices[key] ?? ""}
                    candidates={others}
                    label={`Change for ${describeUsage(usage)}`}
                    placeholder="Choose"
                    onChange={(value) => {
                      setChoices((current) => ({ ...current, [key]: value }))
                    }}
                  />
                </li>
              )
            })}
          </ul>
          <p className="text-xs text-muted-foreground">
            Unbind leaves the key empty, or removes the step or node from its workflow.
            {undecided > 0 &&
              ` ${undecided === 1 ? "1 reference needs" : `${undecided} references need`} a choice.`}
          </p>
        </div>
      )}
    </ModalShell>
  )
}
