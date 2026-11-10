import { describeUsage } from "@/lib/format"
import type { Usage } from "@/lib/types"

export function UsageList({ usages }: { usages: Usage[] }) {
  if (usages.length === 0) {
    return <p className="text-sm text-muted-foreground">Not used in any workflow or layout.</p>
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
