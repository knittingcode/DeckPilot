import { ExternalLink, Trash2 } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ModalShell } from "@/components/modal-shell"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { describeReference, isDirect, siteKey } from "@/lib/action-references"
import { i18n, useTranslation } from "@/i18n"
import { api, errorMessage } from "@/lib/api"
import type { ActionInstance, ActionReference, ReferenceSite } from "@/lib/types"

/** Every place that uses an action, each with a way to open it. */
export function ReferenceList({
  references,
  onOpen,
}: {
  references: ActionReference[]
  onOpen?: (reference: ActionReference) => void
}) {
  const { t } = useTranslation()
  if (references.length === 0) {
    return <p className="text-sm text-muted-foreground">{t("actions:references.notUsed")}</p>
  }
  return (
    <ul className="grid gap-1 text-sm" aria-label={t("actions:references.label")}>
      {references.map((reference) => {
        const text = describeReference(reference)
        return (
          <li key={siteKey(reference.site)} className="flex items-center justify-between gap-2">
            <span className="min-w-0 text-muted-foreground">{text}</span>
            {onOpen && (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                aria-label={t("actions:references.openLabel", { reference: text })}
                onClick={() => {
                  onOpen(reference)
                }}
              >
                <ExternalLink /> {t("actions:references.open")}
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

function ChoiceSelect({
  value,
  candidates,
  canUnbind,
  label,
  placeholder,
  onChange,
}: {
  value: string
  candidates: ActionInstance[]
  canUnbind: boolean
  label: string
  placeholder: string
  onChange: (value: string) => void
}) {
  const { t } = useTranslation()
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger className="w-full sm:w-64" aria-label={label}>
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        {candidates.map((a) => (
          <SelectItem key={a.id} value={`${REPLACE}${a.id}`}>
            {t("actions:references.replaceWith", { name: a.name })}
          </SelectItem>
        ))}
        {canUnbind && <SelectItem value={UNBIND}>{t("actions:references.unbind")}</SelectItem>}
      </SelectContent>
    </Select>
  )
}

/** A message as a sentence: trimmed, starting upper case, ending with a full stop. */
function sentence(message: string): string {
  const text = message.trim()
  if (!text) return ""
  const capital = text.charAt(0).toUpperCase() + text.slice(1)
  return /[.!?…]$/.test(capital) ? capital : `${capital}.`
}

/** What failed, the backend's reason and what to do, as one line. */
function failureMessage(what: string, error: unknown, next: string): string {
  return i18n.t("errors:failure.withDetail", {
    what,
    detail: `${sentence(errorMessage(error))} ${sentence(next)}`,
  })
}

/**
 * Deletes an action. While it is used, every key binding, step and node
 * needs a decision first: use another action instead, or unbind it (keys
 * and steps; a graph node can only be replaced). Triggers follow their
 * workflow and are only listed. Nothing changes until the user confirms,
 * and no use is changed without a decision for it.
 */
export function DeleteActionDialog({
  profileId,
  action,
  references,
  candidates,
  onClose,
  onDeleted,
  onOpen,
}: {
  profileId: string
  action: ActionInstance
  references: ActionReference[]
  /** The actions a use can be pointed at instead. */
  candidates: ActionInstance[]
  onClose: () => void
  onDeleted: () => void
  /** Opens where a use is, e.g. a graph node to remove it in the workflow editor. */
  onOpen?: (reference: ActionReference) => void
}) {
  const { t } = useTranslation()
  const [choices, setChoices] = useState<Record<string, string>>({})
  const [all, setAll] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const others = candidates.filter((a) => a.id !== action.id)
  const direct = references.filter(isDirect)
  const indirect = references.filter((r) => !isDirect(r))
  const undecided = direct.filter((r) => !choices[siteKey(r.site)]).length

  const confirm = async () => {
    setBusy(true)
    setError(null)
    // What failed, for the message, and whether earlier steps changed anything.
    let step = t("actions:references.failure.notReplaced")
    let changed = false
    try {
      // Replacements first: they keep step positions, which unbinding changes.
      const unbind: ReferenceSite[] = []
      const replace = new Map<string, ReferenceSite[]>()
      for (const { site } of direct) {
        const choice = choices[siteKey(site)] ?? ""
        if (choice === UNBIND) unbind.push(site)
        else if (choice.startsWith(REPLACE)) {
          const target = choice.slice(REPLACE.length)
          replace.set(target, [...(replace.get(target) ?? []), site])
        }
      }
      let cleared = 0
      for (const [target, sites] of replace) {
        const result = await api.replaceActionReferences(profileId, action.id, target, {
          type: "sites",
          sites,
        })
        cleared += result.clearedSettings.length
        changed = true
      }
      if (unbind.length > 0) {
        step = t("actions:references.failure.notUnbound")
        await api.unbindActionReferences(profileId, action.id, { type: "sites", sites: unbind })
        changed = true
      }
      step = t("actions:references.failure.notDeleted")
      await api.deleteAction(profileId, action.id)
      if (cleared > 0) {
        toast.warning(t("actions:references.clearedSettings", { count: cleared }))
      }
      onDeleted()
    } catch (e) {
      setError(
        failureMessage(
          step,
          e,
          changed
            ? t("actions:references.failure.changesKept")
            : t("actions:references.failure.nothingChanged"),
        ),
      )
    } finally {
      setBusy(false)
    }
  }

  return (
    <ModalShell
      presentation="dialog"
      open
      size="medium"
      icon={Trash2}
      title={t("actions:references.delete.title", { name: action.name })}
      description={
        direct.length === 0
          ? t("actions:references.delete.unused")
          : t("actions:references.delete.used", { count: direct.length })
      }
      error={error}
      onClose={onClose}
      secondary={{ label: t("common:actions.cancel"), onClick: onClose }}
      primary={{
        label:
          direct.length === 0
            ? t("common:actions.delete")
            : t("actions:references.delete.applyAndDelete"),
        loadingLabel: t("actions:references.delete.deleting"),
        variant: "destructive",
        loading: busy,
        disabled: undecided > 0,
        onClick: () => void confirm(),
      }}
    >
      {direct.length > 0 && (
        <div className="grid gap-4">
          <div className="grid gap-2">
            <p className="text-sm font-medium">{t("actions:references.all")}</p>
            <ChoiceSelect
              value={all}
              candidates={others}
              canUnbind
              label={t("actions:references.forAll")}
              placeholder={t("actions:references.chooseForAll")}
              onChange={(value) => {
                setAll(value)
                // Graph nodes cannot be unbound; they keep their own choice.
                setChoices((current) =>
                  Object.fromEntries(
                    direct.map((r) => {
                      const key = siteKey(r.site)
                      const keep = value === UNBIND && r.site.type === "node"
                      return [key, keep ? (current[key] ?? "") : value]
                    }),
                  ),
                )
              }}
            />
          </div>
          <ul className="grid gap-3" aria-label={t("actions:references.toChange")}>
            {direct.map((reference) => {
              const key = siteKey(reference.site)
              const text = describeReference(reference)
              return (
                <li key={key} className="grid gap-2 sm:flex sm:items-center sm:justify-between">
                  <span className="text-sm">{text}</span>
                  {reference.site.type === "node" && onOpen && (
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      aria-label={t("actions:references.openInEditorLabel", { reference: text })}
                      onClick={() => {
                        onOpen(reference)
                      }}
                    >
                      <ExternalLink /> {t("actions:references.openInEditor")}
                    </Button>
                  )}
                  <ChoiceSelect
                    value={choices[key] ?? ""}
                    candidates={others}
                    canUnbind={reference.site.type !== "node"}
                    label={t("actions:references.changeFor", { reference: text })}
                    placeholder={t("actions:references.choose")}
                    onChange={(value) => {
                      setChoices((current) => ({ ...current, [key]: value }))
                    }}
                  />
                </li>
              )
            })}
          </ul>
          <p className="text-xs text-muted-foreground">
            {t("actions:references.unbindHint")}
            {undecided > 0 && ` ${t("actions:references.undecided", { count: undecided })}`}
          </p>
          {indirect.length > 0 && (
            <div className="grid gap-1">
              <p className="text-sm font-medium">{t("actions:references.triggers")}</p>
              <ul
                className="grid gap-1 text-sm text-muted-foreground"
                aria-label={t("actions:references.triggers")}
              >
                {indirect.map((reference) => (
                  <li key={siteKey(reference.site)}>{describeReference(reference)}</li>
                ))}
              </ul>
              <p className="text-xs text-muted-foreground">
                {t("actions:references.triggersHint")}
              </p>
            </div>
          )}
        </div>
      )}
    </ModalShell>
  )
}
