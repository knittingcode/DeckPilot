import type { LivePreview } from "@/lib/types"
import { cn } from "@/lib/utils"

import { type LiveStatus, liveValuesText } from "./use-live-key-states"

/** One quiet line under the page navigation about the live values. */
export function LiveValuesNote({ status, live }: { status: LiveStatus; live: LivePreview | null }) {
  const text = liveValuesText(status, live)
  if (!text) return null
  const current = status === "live"
  return (
    <p
      className="flex items-center gap-1.5 text-xs text-muted-foreground"
      role="status"
      data-slot="live-values"
      data-state={status}
    >
      <span
        aria-hidden
        className={cn(
          "size-1.5 rounded-full",
          current ? "bg-emerald-500" : "bg-muted-foreground/50",
        )}
      />
      {text}
    </p>
  )
}
