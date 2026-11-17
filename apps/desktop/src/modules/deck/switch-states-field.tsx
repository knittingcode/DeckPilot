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
      {current && !known(current) && <SelectItem value={current}>Missing (deleted)</SelectItem>}
      {offered.length > 0 && (
        <SelectGroup>
          <SelectLabel>Actions</SelectLabel>
          {offered.map((a) => (
            <SelectItem key={a.id} value={`action:${a.id}`}>
              {a.name}
            </SelectItem>
          ))}
        </SelectGroup>
      )}
      {workflows.length > 0 && (
        <SelectGroup>
          <SelectLabel>Workflows and Multi Actions</SelectLabel>
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
              <span className="text-sm text-destructive">Unreadable: {line.raw}</span>
            ) : (
              <>
                <Select
                  value={line.target}
                  onValueChange={(target) => {
                    change(index, { target })
                  }}
                >
                  <SelectTrigger className="w-48" aria-label={`Runs in state ${index + 1}`}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>{targets(line.target)}</SelectContent>
                </Select>
                <Input
                  className="h-8 w-32"
                  maxLength={32}
                  placeholder="Title"
                  aria-label={`Title of state ${index + 1}`}
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
                  <SelectTrigger className="w-28" aria-label={`Glyph of state ${index + 1}`}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NO_GLYPH}>No glyph</SelectItem>
                    {GLYPHS.map((glyph) => (
                      <SelectItem key={glyph} value={glyph}>
                        {glyph}
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
              aria-label={`Move state ${index + 1} up`}
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
              aria-label={`Move state ${index + 1} down`}
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
              aria-label={`Remove state ${index + 1}`}
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
          <SelectTrigger className="w-56" aria-label="Add a state">
            <SelectValue placeholder="Add a state" />
          </SelectTrigger>
          <SelectContent>{targets()}</SelectContent>
        </Select>
      )}
      <p className="text-xs text-muted-foreground">
        Each press runs the current state and, when that run succeeds, moves to the next one; after
        the last it starts again. {MIN_STATES} to {MAX_STATES} states.
        {lines.length < MIN_STATES && " Add at least two."}
      </p>
    </div>
  )
}
