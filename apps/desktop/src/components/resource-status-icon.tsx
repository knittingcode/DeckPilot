import { useTranslation } from "@/i18n"
import { STATUS } from "@/lib/resource-status"
import type { ResourceStatus } from "@/lib/types"
import { cn } from "@/lib/utils"

/** The mark of a resource check's result, named for screen readers. */
export function StatusIcon({ status }: { status: ResourceStatus }) {
  const { t } = useTranslation()
  const { icon: Icon, className, label } = STATUS[status]
  return <Icon className={cn("size-3.5 shrink-0", className)} aria-label={t(label)} />
}
