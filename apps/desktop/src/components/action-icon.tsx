import { Zap } from "lucide-react"

import { ACTION_ICONS } from "@/lib/icons"

export function ActionIcon({
  name,
  className,
}: {
  name: string | null | undefined
  className?: string
}) {
  const Icon = (name && ACTION_ICONS[name]) || Zap
  return <Icon className={className} aria-hidden />
}
