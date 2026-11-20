import { useState } from "react"

import { Button } from "@/components/ui/button"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { ActionIcon } from "@/components/action-icon"
import { useTranslation } from "@/i18n"
import { ACTION_ICONS } from "@/lib/icons"
import { cn } from "@/lib/utils"

export function IconPicker({
  id,
  value,
  onChange,
}: {
  id?: string
  value: string | null
  onChange: (icon: string | null) => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button id={id} variant="outline" className="w-full justify-start gap-2">
          <ActionIcon name={value} className="size-4" />
          <span className="text-muted-foreground">{value ?? t("editor:icons.pickerDefault")}</span>
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-72" align="start">
        <div className="grid grid-cols-8 gap-1" role="listbox" aria-label={t("editor:icons.list")}>
          {Object.keys(ACTION_ICONS).map((name) => (
            <button
              key={name}
              type="button"
              role="option"
              aria-selected={value === name}
              title={name}
              className={cn(
                "flex size-7 items-center justify-center rounded-md hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                value === name && "bg-primary text-primary-foreground hover:bg-primary",
              )}
              onClick={() => {
                onChange(name)
                setOpen(false)
              }}
            >
              <ActionIcon name={name} className="size-4" />
            </button>
          ))}
        </div>
        <Button
          variant="ghost"
          size="sm"
          className="mt-2 w-full"
          onClick={() => {
            onChange(null)
            setOpen(false)
          }}
        >
          {t("editor:icons.pickerUseDefault")}
        </Button>
      </PopoverContent>
    </Popover>
  )
}
