import { Braces } from "lucide-react"
import { useId, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { useTranslation } from "@/i18n"
import type { ParameterKind } from "@/lib/types"
import { cn } from "@/lib/utils"
import { insertedReference } from "@/lib/variable-catalog"
import { describeContext } from "@/lib/variables"

import { type VariableScope, useVariableScope } from "./scope"
import { VariableResults } from "./variable-results"
import { activeDescendant, useVariableBrowser } from "./use-variables"

/**
 * The button beside a field that opens the picker: search, groups,
 * keyboard navigation and the inspector of the highlighted reference.
 * `onPick` gets the reference without braces. Hidden while the agent cannot
 * resolve variables.
 */
export function VariablePicker({
  kind,
  fieldLabel,
  onPick,
  disabled,
  label,
  className,
}: {
  /** The field's kind: only compatible references are listed. */
  kind: ParameterKind | undefined
  fieldLabel: string
  onPick: (reference: string) => void
  disabled?: boolean
  /** Accessible name of the button; by default "Insert a variable into …". */
  label?: string
  className?: string
}) {
  const { t } = useTranslation()
  const scope = useVariableScope()
  const [open, setOpen] = useState(false)
  if (!scope?.live) return null
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="icon"
          className={cn("size-9 shrink-0", className)}
          disabled={disabled}
          aria-label={label ?? t("editor:variables.insertInto", { field: fieldLabel })}
          title={t("editor:variables.insert")}
        >
          <Braces />
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        className="w-[36rem] max-w-[90vw] p-0"
        aria-label={t("editor:variables.forField", { field: fieldLabel })}
      >
        <PickerBody
          scope={scope}
          kind={kind}
          onPick={(reference) => {
            setOpen(false)
            onPick(reference)
          }}
        />
      </PopoverContent>
    </Popover>
  )
}

function PickerBody({
  scope,
  kind,
  onPick,
}: {
  scope: VariableScope
  kind: ParameterKind | undefined
  onPick: (reference: string) => void
}) {
  const { t } = useTranslation()
  const listId = useId()
  const [query, setQuery] = useState("")
  const browser = useVariableBrowser(scope, query, kind)
  return (
    <div className="grid gap-0">
      <div className="grid gap-1 border-b p-2">
        <Input
          autoFocus
          role="combobox"
          aria-label={t("editor:variables.search")}
          aria-expanded
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={activeDescendant(listId, browser)}
          placeholder={t("editor:variables.searchPlaceholder")}
          className="h-8 text-xs"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
          }}
          onKeyDown={(e) => {
            browser.handleKey(e, (entry) => {
              onPick(insertedReference(entry))
            })
          }}
        />
        <p
          className="text-[0.7rem] text-muted-foreground"
          aria-label={t("editor:variables.previewContext")}
        >
          {t("editor:variables.valuesFor", {
            scope: describeContext(scope.context, scope.names),
          })}
        </p>
      </div>
      <VariableResults
        listId={listId}
        scope={scope}
        kind={kind}
        browser={browser}
        onPick={(entry) => {
          onPick(insertedReference(entry))
        }}
      />
    </div>
  )
}
