import { describeUsage } from "@/lib/format"
import type { Usage } from "@/lib/types"
import { useTranslation } from "@/i18n"

export function UsageList({ usages }: { usages: Usage[] }) {
  const { t } = useTranslation()
  if (usages.length === 0) {
    return <p className="text-sm text-muted-foreground">{t("settings:usage.none")}</p>
  }
  return (
    <ul className="grid gap-1 text-sm">
      {usages.map((usage, index) => (
        <li key={index} className="text-muted-foreground">
          {describeUsage(usage)}
        </li>
      ))}
    </ul>
  )
}
