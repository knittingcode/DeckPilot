import { cn } from "@/lib/utils"

import { formatMs } from "./describe"
import { type RowStatus, STATUS_LABEL, type Timeline, barGeometry } from "./run-debug"

const BAR: Record<RowStatus, string> = {
  running: "bg-primary animate-pulse",
  succeeded: "bg-emerald-500 dark:bg-emerald-400",
  failed: "bg-destructive",
  skipped: "bg-muted-foreground/30",
  cancelled: "bg-muted-foreground/60",
}

/**
 * Start and duration of every node relative to the run. Nodes that ran at
 * the same time (parallel branches) are marked and drawn on their own lane.
 */
export function RunTimeline({
  timeline,
  selected,
  onSelect,
}: {
  timeline: Timeline
  selected: string | null
  onSelect: (node: string) => void
}) {
  return (
    <div className="grid gap-1">
      <ol className="grid gap-0.5" aria-label="Execution timeline">
        {timeline.rows.map((row) => {
          const { left, width } = barGeometry(row, timeline.spanMs)
          const description = [
            STATUS_LABEL[row.status],
            `${row.estimated ? "started about" : "started"} ${formatMs(row.startMs)} in`,
            row.status === "running"
              ? `running for ${formatMs(row.durationMs)}`
              : `took ${formatMs(row.durationMs)}`,
            row.parallel ? "in parallel" : "",
            row.attempts > 1 ? `${row.attempts} attempts` : "",
          ]
            .filter(Boolean)
            .join(", ")
          return (
            <li key={row.node}>
              <button
                type="button"
                aria-pressed={selected === row.node}
                aria-label={`${row.title}: ${description}`}
                title={`${row.title}: ${description}`}
                onClick={() => {
                  onSelect(row.node)
                }}
                className={cn(
                  "grid w-full grid-cols-[6.5rem_minmax(0,1fr)] items-center gap-2 rounded px-1 py-0.5 text-left text-[0.6875rem] hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                  selected === row.node && "bg-accent",
                )}
              >
                <span className="flex min-w-0 items-center gap-1">
                  {row.parallel && (
                    <span
                      className="shrink-0 font-mono text-muted-foreground"
                      aria-hidden
                      title={`Lane ${row.lane + 1}`}
                    >
                      ∥
                    </span>
                  )}
                  <span className="truncate">{row.title}</span>
                </span>
                <span className="relative h-2.5 rounded-sm bg-muted" aria-hidden>
                  <span
                    className={cn(
                      "absolute inset-y-0 rounded-sm",
                      BAR[row.status],
                      row.estimated && "opacity-60",
                    )}
                    style={{ left: `${left}%`, width: `${width}%` }}
                  />
                </span>
              </button>
            </li>
          )
        })}
      </ol>
      <div
        className="grid grid-cols-[6.5rem_minmax(0,1fr)] gap-2 px-1 text-[0.625rem] text-muted-foreground"
        aria-hidden
      >
        <span />
        <span className="flex justify-between">
          <span>0</span>
          <span>{formatMs(timeline.spanMs)}</span>
        </span>
      </div>
    </div>
  )
}
