import { Plus, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useTranslation } from "@/i18n"
import type { ValueKind, WorkflowVariable } from "@/lib/types"

import { MAX_WORKFLOW_VALUES } from "./graph"
import { VALUE_KIND_LABEL } from "./mapping"

const KINDS = Object.keys(VALUE_KIND_LABEL) as ValueKind[]

function nextKey(taken: WorkflowVariable[]) {
  for (let n = 1; ; n++) {
    const key = `var_${n}`
    if (!taken.some((v) => v.key === key)) return key
  }
}

/**
 * The open workflow's variables: values that live for one run, set by Set
 * variable nodes. The engine validates them with the graph.
 */
export function VariablesEditor({
  variables,
  onChange,
}: {
  variables: WorkflowVariable[]
  onChange: (variables: WorkflowVariable[]) => void
}) {
  const { t } = useTranslation()
  const set = (index: number, change: Partial<WorkflowVariable>) => {
    onChange(variables.map((v, i) => (i === index ? { ...v, ...change } : v)))
  }
  return (
    <section className="grid gap-3 border-t px-4 py-3" aria-label={t("workflows:variables.title")}>
      <div className="grid gap-1">
        <h3 className="text-xs font-medium text-muted-foreground uppercase">
          {t("workflows:variables.title")}
        </h3>
        <p className="text-xs text-muted-foreground">{t("workflows:variables.description")}</p>
      </div>
      {variables.map((variable, index) => (
        <div
          key={index}
          className="grid grid-cols-[minmax(0,1fr)] gap-1.5 rounded-md border p-2"
          role="group"
          aria-label={t("workflows:variables.group", { key: variable.key })}
        >
          <div className="flex items-center gap-1">
            <Input
              aria-label={t("workflows:variables.key")}
              aria-invalid={!/^[A-Za-z0-9_]{1,64}$/.test(variable.key)}
              className="h-7 min-w-0 flex-1 font-mono text-xs"
              value={variable.key}
              onChange={(e) => {
                set(index, { key: e.target.value })
              }}
            />
            <Select
              value={variable.kind}
              onValueChange={(kind) => {
                set(index, { kind: kind as ValueKind })
              }}
            >
              <SelectTrigger
                className="h-7 w-28 text-xs"
                aria-label={t("workflows:variables.type")}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {KINDS.map((kind) => (
                  <SelectItem key={kind} value={kind}>
                    {VALUE_KIND_LABEL[kind]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button
              variant="ghost"
              size="icon"
              className="size-7 shrink-0"
              aria-label={t("workflows:variables.remove", { key: variable.key })}
              onClick={() => {
                onChange(variables.filter((_, i) => i !== index))
              }}
            >
              <X />
            </Button>
          </div>
          <Input
            aria-label={t("workflows:variables.label")}
            className="h-7 text-xs"
            placeholder={t("workflows:interface.labelPlaceholder")}
            value={variable.label ?? ""}
            onChange={(e) => {
              set(index, { label: e.target.value || undefined })
            }}
          />
          <Input
            aria-label={t("workflows:variables.initial")}
            className="h-7 text-xs"
            placeholder={t("workflows:variables.initialPlaceholder")}
            value={variable.initial ?? ""}
            onChange={(e) => {
              set(index, { initial: e.target.value === "" ? undefined : e.target.value })
            }}
          />
          <div className="flex items-center gap-1.5">
            <Checkbox
              id={`variable-sensitive-${index}`}
              checked={Boolean(variable.sensitive)}
              onCheckedChange={(checked) => {
                set(index, { sensitive: checked === true || undefined })
              }}
            />
            <Label htmlFor={`variable-sensitive-${index}`} className="text-xs font-normal">
              {t("workflows:interface.sensitiveNotKept")}
            </Label>
          </div>
        </div>
      ))}
      <Button
        variant="outline"
        size="sm"
        disabled={variables.length >= MAX_WORKFLOW_VALUES}
        onClick={() => {
          onChange([...variables, { key: nextKey(variables), kind: "text" }])
        }}
      >
        <Plus /> {t("workflows:variables.add")}
      </Button>
    </section>
  )
}
