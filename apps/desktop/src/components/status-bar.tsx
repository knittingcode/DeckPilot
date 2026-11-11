import {
  Bell,
  BellDot,
  CircleAlert,
  CircleCheck,
  CircleSlash,
  Ellipsis,
  Info,
  Loader2,
  TriangleAlert,
  X,
} from "lucide-react"
import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { type StatusStores, useSnapshot } from "@/hooks/use-status"
import { formatTimestamp } from "@/lib/format"
import {
  type AppNotification,
  NotificationStore,
  type Operation,
  OperationStore,
  type Severity,
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
 * context on the left, background operations in the center, updates and
 * notifications on the right. It renders what core services and enabled
 * modules registered and imports no module.
 */
export function AppStatusBar({ stores }: { stores: StatusStores }) {
  const items = useSnapshot(stores.items)
  const operations = useSnapshot(stores.operations)
  const notifications = useSnapshot(stores.notifications)

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
      aria-label="Status bar"
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
        <Notifications store={stores.notifications} notifications={notifications} />
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
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="More status"
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

function operationText(operation: Operation) {
  if (operation.status === "failed") return `${operation.title} failed`
  if (operation.status === "succeeded") return `${operation.title} finished`
  if (operation.status === "cancelled") return `${operation.title} cancelled`
  if (operation.total !== undefined && operation.completed !== undefined) {
    return `${operation.title} (${operation.completed}/${operation.total})`
  }
  return operation.title
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
          aria-label={`Background activity: ${operationText(primary)}${others > 0 ? `, ${others} more` : ""}`}
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
          Background activity
        </h2>
        <ul className="grid max-h-72 gap-px overflow-y-auto">
          {ordered.map((operation) => (
            <li key={operation.id} className="grid gap-1 px-3 py-2">
              <div className="flex items-center gap-2 [&_svg]:size-3.5">
                <OperationIcon operation={operation} />
                <span className="flex-1 truncate">{operation.title}</span>
                {operation.cancel && operation.status === "running" && (
                  <Button variant="ghost" size="sm" className="h-6 px-2" onClick={operation.cancel}>
                    Cancel
                  </Button>
                )}
              </div>
              {operation.status === "running" && operation.progress !== undefined && (
                <MiniProgress percent={operation.progress} />
              )}
              <p className="text-xs text-muted-foreground">
                {operation.error ??
                  (operation.status === "running"
                    ? `Started ${formatTimestamp(Math.floor(operation.startedAt / 1000))}`
                    : `Finished ${formatTimestamp(Math.floor((operation.finishedAt ?? 0) / 1000))}`)}
              </p>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  )
}

const SEVERITY: Record<Severity, { icon: typeof Info; className: string }> = {
  info: { icon: Info, className: "text-muted-foreground" },
  warning: { icon: TriangleAlert, className: "text-amber-600 dark:text-amber-400" },
  error: { icon: CircleAlert, className: "text-destructive" },
}

function Notifications({
  store,
  notifications,
}: {
  store: NotificationStore
  notifications: AppNotification[]
}) {
  const [open, setOpen] = useState(false)
  const unread = notifications.filter((n) => !n.read)
  const worst = unread.some((n) => n.severity === "error")
    ? "error"
    : unread.some((n) => n.severity === "warning")
      ? "warning"
      : null
  const list = NotificationStore.newestFirst(notifications)
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        // Seen once the list closes.
        if (!next) store.markAllRead()
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          className={SEGMENT}
          aria-label={
            unread.length > 0 ? `Notifications, ${unread.length} unread` : "Notifications"
          }
        >
          {unread.length > 0 ? (
            <BellDot className={worst ? SEVERITY[worst].className : undefined} />
          ) : (
            <Bell />
          )}
          {unread.length > 0 && <span className="tabular-nums">{unread.length}</span>}
        </button>
      </PopoverTrigger>
      <PopoverContent
        side="top"
        align="end"
        sideOffset={6}
        collisionPadding={8}
        className="w-96 p-0 text-sm"
      >
        <div className="flex items-center border-b px-3 py-2">
          <h2 className="flex-1 text-xs font-medium text-muted-foreground uppercase">
            Notifications
          </h2>
          {list.length > 0 && (
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-2 text-xs"
              onClick={() => {
                store.clear()
              }}
            >
              <X /> Clear
            </Button>
          )}
        </div>
        {list.length === 0 ? (
          <p className="px-3 py-6 text-center text-sm text-muted-foreground">No notifications</p>
        ) : (
          <ul className="grid max-h-80 overflow-y-auto" aria-label="Notifications">
            {list.map((n) => {
              const { icon: Icon, className } = SEVERITY[n.severity]
              return (
                <li
                  key={n.key}
                  className={cn(
                    "flex gap-2 border-b px-3 py-2 last:border-b-0",
                    !n.read && "bg-accent/40",
                  )}
                >
                  <Icon
                    className={cn("mt-0.5 size-4 shrink-0", className)}
                    aria-label={n.severity}
                  />
                  <div className="grid min-w-0 flex-1 gap-0.5">
                    <span className="font-medium">
                      {n.title}
                      {n.count > 1 && (
                        <span className="ml-1 text-xs font-normal text-muted-foreground">
                          ×{n.count}
                        </span>
                      )}
                    </span>
                    {n.message && (
                      <span className="text-xs text-muted-foreground">{n.message}</span>
                    )}
                    <span className="text-xs text-muted-foreground">
                      {formatTimestamp(Math.floor(n.at / 1000))}
                    </span>
                  </div>
                </li>
              )
            })}
          </ul>
        )}
      </PopoverContent>
    </Popover>
  )
}
