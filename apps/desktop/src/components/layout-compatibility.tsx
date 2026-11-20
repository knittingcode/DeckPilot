import { AlertTriangle, Check, CircleAlert } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { i18n, useTranslation } from "@/i18n"
import { fitsModel, hasUnknownModel } from "@/lib/shared-layouts"
import type { LayoutCompatibility, SharedLayoutSummary } from "@/lib/types"

function line(entry: LayoutCompatibility): string {
  return i18n.t("layouts:compatibility.line", {
    model: entry.modelName,
    explanation: entry.explanation,
  })
}

/** The device of a layout: its model's name, or the id with a warning badge. */
export function LayoutDevice({ layout }: { layout: SharedLayoutSummary }) {
  const { t } = useTranslation()
  if (!hasUnknownModel(layout)) return <span className="truncate">{layout.modelName}</span>
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <span className="truncate font-mono text-xs">{layout.model}</span>
      <Badge
        variant="outline"
        className="border-amber-500/50 text-amber-700 dark:text-amber-400"
        title={layout.compatibility[0]?.explanation ?? t("layouts:compatibility.notInCatalog")}
      >
        <AlertTriangle aria-hidden />
        {t("layouts:compatibility.unknownModel")}
      </Badge>
    </span>
  )
}

/**
 * A compact compatibility indicator: how many other models the layout fits
 * and misses; the tooltip lists each model with the explanation.
 */
export function LayoutCompatibilitySummary({ layout }: { layout: SharedLayoutSummary }) {
  const { t } = useTranslation()
  const [own, ...others] = layout.compatibility
  const fits = others.filter((c) => fitsModel(c.fit)).length
  const misses = others.filter((c) => c.fit.type === "missing").length
  const lines = layout.compatibility.map(line)
  const label =
    others.length === 0
      ? t("layouts:profileLayouts.compatibility.ownOnly")
      : fits > 0 && misses > 0
        ? t("layouts:profileLayouts.compatibility.fitsMisses", { fits, misses })
        : fits > 0
          ? t("layouts:profileLayouts.compatibility.fits", { fits })
          : misses > 0
            ? t("layouts:profileLayouts.compatibility.misses", { misses })
            : t("layouts:profileLayouts.compatibility.unknown")
  const unknown = own?.fit.type === "unknownModel"
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          // Focusable so the tooltip is reachable from the keyboard.
          tabIndex={0}
          role="note"
          aria-label={t("layouts:profileLayouts.compatibility.label", {
            lines: lines.join("; ") || t("layouts:compatibility.unknown"),
          })}
          className="inline-flex items-center gap-1 rounded text-xs text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          {unknown ? (
            <CircleAlert className="size-3.5 text-amber-600" aria-hidden />
          ) : misses > 0 ? (
            <CircleAlert className="size-3.5" aria-hidden />
          ) : (
            <Check className="size-3.5 text-emerald-600" aria-hidden />
          )}
          {label}
        </span>
      </TooltipTrigger>
      <TooltipContent>
        <ul className="grid gap-0.5">
          {lines.map((text) => (
            <li key={text}>{text}</li>
          ))}
        </ul>
      </TooltipContent>
    </Tooltip>
  )
}
