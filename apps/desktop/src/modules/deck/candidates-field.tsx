import { X } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useTranslation } from "@/i18n"
import type { ActionInstance, Workflow } from "@/lib/types"

import {
  MAX_WEIGHT,
  RANDOM_TYPE,
  type CandidateLine,
  formatCandidates,
  parseCandidates,
} from "./candidates"

/**
 * The candidates of a Random Action: rows of a saved action or workflow and
 * a weight, stored as the text the agent reads. Other Random Actions are
 * not offered (picks do not nest), and something that no longer exists
 * stays in its row, shown as missing.
 */
export function CandidatesField({
  id,
  value,
  actions,
  workflows,
  onChange,
}: {
  id: string
  value: string
  actions: Pick<ActionInstance, "id" | "name" | "actionType">[]
  workflows: Pick<Workflow, "id" | "name">[]
  onChange: (text: string) => void
}) {
  const { t } = useTranslation()
  const lines = parseCandidates(value)
  const offered = actions.filter((a) => a.actionType !== RANDOM_TYPE)
  const exists = (target: string) => {
    const [kind, ref] = target.split(":")
    return kind === "action"
      ? actions.some((a) => a.id === ref)
      : workflows.some((w) => w.id === ref)
  }
  const set = (next: CandidateLine[]) => {
    onChange(formatCandidates(next))
  }
  const options = (current?: string) => (
    <>
      {current && !exists(current) && (
        <SelectItem value={current}>{t("actions:deck.missingDeleted")}</SelectItem>
      )}
      {offered.length > 0 && (
        <SelectGroup>
          <SelectLabel>{t("actions:deck.groupActions")}</SelectLabel>
          {offered.map((a) => (
            <SelectItem key={a.id} value={`action:${a.id}`}>
              {a.name}
            </SelectItem>
          ))}
        </SelectGroup>
      )}
      {workflows.length > 0 && (
        <SelectGroup>
          <SelectLabel>{t("actions:deck.groupWorkflows")}</SelectLabel>
          {workflows.map((w) => (
            <SelectItem key={w.id} value={`workflow:${w.id}`}>
              {w.name}
            </SelectItem>
          ))}
        </SelectGroup>
      )}
    </>
  )
  return (
    <div className="grid gap-2" id={id}>
      {lines.length === 0 && (
        <p className="text-sm text-muted-foreground">{t("actions:deck.candidates.empty")}</p>
      )}
      <ol className="grid gap-2">
        {lines.map((line, index) => (
          <li key={index} className="flex flex-wrap items-center gap-2">
            {line.raw !== undefined ? (
              <span className="text-sm text-destructive">
                {t("actions:deck.unreadable", { line: line.raw })}
              </span>
            ) : (
              <>
                <Select
                  value={line.target}
                  onValueChange={(target) => {
                    set(lines.map((l, i) => (i === index ? { ...l, target } : l)))
                  }}
                >
                  <SelectTrigger
                    className="w-56"
                    aria-label={t("actions:deck.candidates.candidate", { number: index + 1 })}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>{options(line.target)}</SelectContent>
                </Select>
                <Input
                  type="number"
                  className="h-8 w-20"
                  min={1}
                  max={MAX_WEIGHT}
                  aria-label={t("actions:deck.candidates.weight", { number: index + 1 })}
                  value={line.weight}
                  onChange={(e) => {
                    const weight = Math.round(Number(e.target.value))
                    if (!Number.isFinite(weight)) return
                    const clamped = Math.min(MAX_WEIGHT, Math.max(1, weight))
                    set(lines.map((l, i) => (i === index ? { ...l, weight: clamped } : l)))
                  }}
                />
              </>
            )}
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("actions:deck.candidates.remove", { number: index + 1 })}
              onClick={() => {
                set(lines.filter((_, i) => i !== index))
              }}
            >
              <X />
            </Button>
          </li>
        ))}
      </ol>
      <Select
        value=""
        onValueChange={(target) => {
          set([...lines, { target, weight: 1 }])
        }}
      >
        <SelectTrigger className="w-56" aria-label={t("actions:deck.candidates.add")}>
          <SelectValue placeholder={t("actions:deck.candidates.addPlaceholder")} />
        </SelectTrigger>
        <SelectContent>{options()}</SelectContent>
      </Select>
      <p className="text-xs text-muted-foreground">{t("actions:deck.candidates.hint")}</p>
    </div>
  )
}
