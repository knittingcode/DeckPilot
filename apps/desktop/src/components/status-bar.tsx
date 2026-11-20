import { CircleAlert, CircleCheck, CircleSlash, Ellipsis, Loader2 } from "lucide-react"
import { useEffect } from "react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { type StatusStores, useSnapshot } from "@/hooks/use-status"
import { i18n, useTranslation } from "@/i18n"
import { formatTimestamp } from "@/lib/format"
import {
  type Operation,
  OperationStore,
  type StatusItem,
  StatusItemStore,
  type StatusTone,
  visibility,
} from "@/lib/status"
import { cn } from "@/lib/utils"

const TONE: Record<StatusTone, string> = {
  neutral: "",
  positive: "text-emerald-600 dark:text-emerald-400",
  warning: "text-amber-600 dark:text-amber-400",
  danger: "text-destructive",
}

/** Inline unless the bar is too narrow for the item's priority. */
const INLINE = {
  always: "inline-flex",
  medium: "hidden @3xl:inline-flex",
  wide: "hidden @5xl:inline-flex",
}
const IN_OVERFLOW = { always: "hidden", medium: "@3xl:hidden", wide: "@5xl:hidden" }

const SEGMENT =
  "inline-flex h-full max-w-64 shrink-0 items-center gap-1.5 rounded-none px-2 text-xs font-normal text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none focus-visible:ring-inset [&_svg]:size-3.5 [&_svg]:shrink-0"

/**
 * The bottom bar of the main window: status of the agent, devices and
 * context on the left, background operations in the center, updates on
 * the right. Notifications are in the toolbar (`NotificationsButton`). It
 * renders what core services and enabled modules registered and imports no
 * module.
 */
export function AppStatusBar({ stores }: { stores: StatusStores }) {
  const { t } = useTranslation()
  const items = useSnapshot(stores.items)
  const operations = useSnapshot(stores.operations)

  // Finished operations disappear after a while.
  const finished = operations.some((o) => o.status !== "running")
  useEffect(() => {
    if (!finished) return
    const timer = setInterval(() => {
      stores.operations.prune()
    }, 1000)
    return () => {
      clearInterval(timer)
    }
  }, [finished, stores.operations])

  const left = StatusItemStore.arrange(items, "left")
  const center = StatusItemStore.arrange(items, "center")
  const right = StatusItemStore.arrange(items, "right")
  const overflow = [...left, ...center, ...right].filter(
    (item) => visibility(item.priority) !== "always",
  )

  return (
    <footer
      data-slot="statusbar"
      aria-label={t("navigation:statusBar.label")}
      className="@container flex h-(--statusbar-size) shrink-0 items-stretch border-t bg-sidebar text-xs select-none"
    >
      <div className="flex min-w-0 items-stretch">
        {left.map((item) => (
          <StatusSegment key={item.id} item={item} />
        ))}
      </div>
      <div className="flex min-w-0 flex-1 items-stretch justify-center">
        <Operations operations={operations} />
        {center.map((item) => (
          <StatusSegment key={item.id} item={item} />
        ))}
      </div>
      <div className="flex min-w-0 items-stretch">
        {right.map((item) => (
          <StatusSegment key={item.id} item={item} />
        ))}
        {overflow.length > 0 && <Overflow items={overflow} />}
      </div>
    </footer>
  )
}

function SegmentContent({ item }: { item: StatusItem }) {
  const Icon = item.progress === true ? Loader2 : item.icon
  return (
    <>
      {Icon && (
        <Icon
          className={cn(item.progress === true && "animate-spin", TONE[item.tone ?? "neutral"])}
        />
      )}
      <span className="truncate">{item.label}</span>
      {typeof item.progress === "number" && <MiniProgress percent={item.progress} />}
      {item.badge !== undefined && (
        <span className="rounded-full bg-muted px-1.5 text-[0.6875rem] leading-4 tabular-nums">
          {item.badge}
        </span>
      )}
    </>
  )
}

function StatusSegment({ item }: { item: StatusItem }) {
  const className = cn(SEGMENT, INLINE[visibility(item.priority)])
  const Detail = item.detail
  if (Detail) {
    return (
      <Popover>
        <PopoverTrigger asChild>
          <button type="button" className={className} title={item.tooltip}>
            <SegmentContent item={item} />
          </button>
        </PopoverTrigger>
        <PopoverContent
          side="top"
          align="start"
          sideOffset={6}
          collisionPadding={8}
          className="w-80 p-3 text-sm"
        >
          <Detail />
        </PopoverContent>
      </Popover>
    )
  }
  if (item.onSelect) {
    return (
      <button type="button" className={className} title={item.tooltip} onClick={item.onSelect}>
        <SegmentContent item={item} />
      </button>
    )
  }
  return (
    <span
      className={cn(className, "hover:bg-transparent hover:text-muted-foreground")}
      title={item.tooltip}
    >
      <SegmentContent item={item} />
    </span>
  )
}

function Overflow({ items }: { items: StatusItem[] }) {
  const { t } = useTranslation()
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={t("navigation:statusBar.more")}
          // Visible while any item is hidden: medium ones below @3xl, wide ones below @5xl.
          className={cn(
            SEGMENT,
            items.some((i) => visibility(i.priority) === "wide") ? "@5xl:hidden" : "@3xl:hidden",
          )}
        >
          <Ellipsis />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="top" align="end" className="min-w-56">
        {items.map((item) => {
          const Icon = item.icon
          return (
            <DropdownMenuItem
              key={item.id}
              className={IN_OVERFLOW[visibility(item.priority)]}
              disabled={!item.onSelect}
              title={item.tooltip}
              onSelect={item.onSelect}
            >
              {Icon && <Icon className={TONE[item.tone ?? "neutral"]} />}
              {item.label}
            </DropdownMenuItem>
          )
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function MiniProgress({ percent }: { percent: number }) {
  const value = Math.max(0, Math.min(100, percent))
  return (
    <span
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value}
      className="h-1 w-16 overflow-hidden rounded-full bg-muted"
    >
      <span className="block h-full bg-primary" style={{ width: `${value}%` }} />
    </span>
  )
}

/** The operation's state in one line (the window's language; its title is the caller's). */
function operationText(operation: Operation) {
  const { title } = operation
  if (operation.status === "failed") return i18n.t("notifications:operations.failed", { title })
  if (operation.status === "succeeded") {
    return i18n.t("notifications:operations.succeeded", { title })
  }
  if (operation.status === "cancelled") {
    return i18n.t("notifications:operations.cancelled", { title })
  }
  if (operation.total !== undefined && operation.completed !== undefined) {
    return i18n.t("notifications:operations.counted", {
      title,
      completed: operation.completed,
      total: operation.total,
    })
  }
  return title
}

function OperationIcon({ operation }: { operation: Operation }) {
  if (operation.status === "failed") return <CircleAlert className="text-destructive" />
  if (operation.status === "succeeded")
    return <CircleCheck className="text-emerald-600 dark:text-emerald-400" />
  if (operation.status === "cancelled") return <CircleSlash className="text-muted-foreground" />
  return <Loader2 className="animate-spin" />
}

/** The primary operation inline; all of them in a popover. */
function Operations({ operations }: { operations: Operation[] }) {
  const { t } = useTranslation()
  const ordered = OperationStore.ordered(operations)
  const primary = ordered[0]
  if (!primary) return null
  const others = ordered.length - 1
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={cn(SEGMENT, "max-w-96")}
          aria-label={
            others > 0
              ? t("notifications:operations.summaryWithMore", {
                  operation: operationText(primary),
                  count: others,
                })
              : t("notifications:operations.summary", { operation: operationText(primary) })
          }
        >
          <OperationIcon operation={primary} />
          <span className="truncate">{operationText(primary)}</span>
          {primary.status === "running" && primary.progress !== undefined && (
            <MiniProgress percent={primary.progress} />
          )}
          {others > 0 && <span className="text-muted-foreground/80">+{others}</span>}
        </button>
      </PopoverTrigger>
      <PopoverContent side="top" sideOffset={6} collisionPadding={8} className="w-96 p-0 text-sm">
        <h2 className="border-b px-3 py-2 text-xs font-medium text-muted-foreground uppercase">
          {t("notifications:operations.title")}
        </h2>
        <ul className="relative grid max-h-72 gap-px overflow-y-auto">
          {ordered.map((operation) => (
            <li key={operation.id} className="grid gap-1 px-3 py-2">
              <div className="flex items-center gap-2 [&_svg]:size-3.5">
                <OperationIcon operation={operation} />
                <span className="flex-1 truncate">{operation.title}</span>
                {operation.cancel && operation.status === "running" && (
                  <Button variant="ghost" size="sm" className="h-6 px-2" onClick={operation.cancel}>
                    {t("common:actions.cancel")}
                  </Button>
                )}
              </div>
              {operation.status === "running" && operation.progress !== undefined && (
                <MiniProgress percent={operation.progress} />
              )}
              <p className="text-xs text-muted-foreground">
                {operation.error ??
                  (operation.status === "running"
                    ? t("notifications:operations.startedAt", {
                        time: formatTimestamp(Math.floor(operation.startedAt / 1000)),
                      })
                    : t("notifications:operations.finishedAt", {
                        time: formatTimestamp(Math.floor((operation.finishedAt ?? 0) / 1000)),
                      }))}
              </p>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  )
}
