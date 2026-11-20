import { ArrowDown, ArrowUp, X } from "lucide-react"

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
import { type TranslationKey, useTranslation } from "@/i18n"
import type { ActionInstance, Workflow } from "@/lib/types"

import { RANDOM_TYPE } from "./candidates"
import {
  GLYPHS,
  MAX_STATES,
  MIN_STATES,
  MULTI_SWITCH_TYPE,
  type SwitchStateLine,
  formatStates,
  parseStates,
} from "./switch-states"

const NO_GLYPH = "none"

const GLYPH_LABELS: Record<(typeof GLYPHS)[number], TranslationKey> = {
  check: "actions:deck.glyph.check",
  cross: "actions:deck.glyph.cross",
  play: "actions:deck.glyph.play",
  pause: "actions:deck.glyph.pause",
  stop: "actions:deck.glyph.stop",
  timer: "actions:deck.glyph.timer",
  bell: "actions:deck.glyph.bell",
  up: "actions:deck.glyph.up",
  down: "actions:deck.glyph.down",
  dot: "actions:deck.glyph.dot",
  circle: "actions:deck.glyph.circle",
  square: "actions:deck.glyph.square",
  diamond: "actions:deck.glyph.diamond",
  triangle: "actions:deck.glyph.triangle",
  star: "actions:deck.glyph.star",
  heart: "actions:deck.glyph.heart",
}

/**
 * The states of a Multi Action Switch: in order, each a saved action or
 * workflow (a Multi Action is a workflow) with the title and glyph the key
 * shows while it is the current state. Random Actions and other switches
 * are not offered (they do not nest); something that no longer exists stays
 * in its row, shown as missing.
 */
export function SwitchStatesField({
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
  const lines = parseStates(value)
  const offered = actions.filter(
    (a) => a.actionType !== RANDOM_TYPE && a.actionType !== MULTI_SWITCH_TYPE,
  )
  const known = (target: string) => {
    const [kind, ref] = target.split(":")
    return kind === "action"
      ? actions.some((a) => a.id === ref)
      : workflows.some((w) => w.id === ref)
  }
  const set = (next: SwitchStateLine[]) => {
    onChange(formatStates(next))
  }
  const change = (index: number, patch: Partial<SwitchStateLine>) => {
    set(lines.map((line, i) => (i === index ? { ...line, ...patch } : line)))
  }
  const move = (from: number, to: number) => {
    const next = [...lines]
    const [line] = next.splice(from, 1)
    if (!line) return
    next.splice(to, 0, line)
    set(next)
  }
  const targets = (current?: string) => (
    <>
      {current && !known(current) && (
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
          <SelectLabel>{t("actions:deck.groupWorkflowsMulti")}</SelectLabel>
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
      <ol className="grid gap-2">
        {lines.map((line, index) => (
          <li key={index} className="flex flex-wrap items-center gap-2 rounded-md border p-2">
            <span className="w-5 text-sm text-muted-foreground tabular-nums">{index + 1}.</span>
            {line.raw !== undefined ? (
              <span className="text-sm text-destructive">
                {t("actions:deck.unreadable", { line: line.raw })}
              </span>
            ) : (
              <>
                <Select
                  value={line.target}
                  onValueChange={(target) => {
                    change(index, { target })
                  }}
                >
                  <SelectTrigger
                    className="w-48"
                    aria-label={t("actions:deck.states.runs", { number: index + 1 })}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>{targets(line.target)}</SelectContent>
                </Select>
                <Input
                  className="h-8 w-32"
                  maxLength={32}
                  placeholder={t("actions:deck.states.titlePlaceholder")}
                  aria-label={t("actions:deck.states.title", { number: index + 1 })}
                  value={line.title}
                  onChange={(e) => {
                    change(index, { title: e.target.value })
                  }}
                />
                <Select
                  value={line.glyph || NO_GLYPH}
                  onValueChange={(glyph) => {
                    change(index, { glyph: glyph === NO_GLYPH ? "" : glyph })
                  }}
                >
                  <SelectTrigger
                    className="w-28"
                    aria-label={t("actions:deck.states.glyph", { number: index + 1 })}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NO_GLYPH}>{t("actions:deck.states.noGlyph")}</SelectItem>
                    {GLYPHS.map((glyph) => (
                      <SelectItem key={glyph} value={glyph}>
                        {t(GLYPH_LABELS[glyph])}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </>
            )}
            <div className="flex-1" />
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("actions:deck.states.moveUp", { number: index + 1 })}
              disabled={index === 0}
              onClick={() => {
                move(index, index - 1)
              }}
            >
              <ArrowUp />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("actions:deck.states.moveDown", { number: index + 1 })}
              disabled={index === lines.length - 1}
              onClick={() => {
                move(index, index + 1)
              }}
            >
              <ArrowDown />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("actions:deck.states.remove", { number: index + 1 })}
              onClick={() => {
                set(lines.filter((_, i) => i !== index))
              }}
            >
              <X />
            </Button>
          </li>
        ))}
      </ol>
      {lines.length < MAX_STATES && (
        <Select
          value=""
          onValueChange={(target) => {
            set([...lines, { target, title: "", glyph: "" }])
          }}
        >
          <SelectTrigger className="w-56" aria-label={t("actions:deck.states.add")}>
            <SelectValue placeholder={t("actions:deck.states.add")} />
          </SelectTrigger>
          <SelectContent>{targets()}</SelectContent>
        </Select>
      )}
      <p className="text-xs text-muted-foreground">
        {t("actions:deck.states.hint", { min: MIN_STATES, count: MAX_STATES })}
        {lines.length < MIN_STATES && ` ${t("actions:deck.states.addAtLeastTwo")}`}
      </p>
    </div>
  )
}
