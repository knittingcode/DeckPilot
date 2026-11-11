import type { LucideIcon } from "lucide-react"
import type { ComponentType } from "react"

/**
 * Shared state of the status bar: status items, background operations and
 * notifications. Core services and enabled modules register into these
 * stores; the status bar renders them and imports no module.
 *
 * Everything shown is data rendered by the app's own components; nothing
 * from IPC is rendered as HTML.
 */

export type StatusPlacement = "left" | "center" | "right"
export type StatusTone = "neutral" | "positive" | "warning" | "danger"

export interface StatusItem {
  /** Stable, namespaced: `core.agent`, `projects.active-project`. */
  id: string
  /** `core` or the owning module's id. */
  owner: string
  placement: StatusPlacement
  /**
   * 0–100. Items of priority 80 and above are always shown; lower ones move
   * into the overflow menu as the window gets narrower.
   */
  priority: number
  label: string
  icon?: LucideIcon
  tooltip?: string
  badge?: string | number
  tone?: StatusTone
  /** `true`: indeterminate; a number: percent. */
  progress?: boolean | number
  /** Opens something, e.g. a settings page. Ignored when `detail` is set. */
  onSelect?: () => void
  /** Content of the popover opened from the item. */
  detail?: ComponentType
}

export type Visibility = "always" | "medium" | "wide"

/** When an item is visible inline (otherwise it is in the overflow menu). */
export function visibility(priority: number): Visibility {
  if (priority >= 80) return "always"
  if (priority >= 50) return "medium"
  return "wide"
}

type Listener = () => void

/** What a value looks like, without callbacks or components. */
function appearance(value: unknown) {
  return JSON.stringify(value, (_, v: unknown) => {
    if (typeof v === "function") return undefined
    if (v && typeof v === "object" && "$$typeof" in v) {
      return (v as { displayName?: string }).displayName ?? "component"
    }
    return v
  })
}

/** Keyed values with change notification; the base of the stores below. */
export class Store<T> {
  protected values = new Map<string, T>()
  private listeners = new Set<Listener>()
  private snapshot: T[] = []

  subscribe = (listener: Listener) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  getSnapshot = () => this.snapshot

  get(id: string) {
    return this.values.get(id)
  }

  protected put(id: string, value: T) {
    const previous = this.values.get(id)
    this.values.set(id, value)
    if (previous === undefined || appearance(previous) !== appearance(value)) this.publish()
  }

  remove(id: string) {
    if (this.values.delete(id)) this.publish()
  }

  protected publish() {
    this.snapshot = [...this.values.values()]
    for (const listener of this.listeners) listener()
  }
}

export class StatusItemStore extends Store<StatusItem> {
  set(item: StatusItem) {
    this.put(item.id, item)
  }

  /** Items of a placement, highest priority first, then by id. */
  static arrange(items: StatusItem[], placement: StatusPlacement) {
    return items
      .filter((item) => item.placement === placement)
      .sort((a, b) => b.priority - a.priority || a.id.localeCompare(b.id))
  }
}

export type OperationStatus = "running" | "succeeded" | "failed" | "cancelled"

export interface Operation {
  /** Stable for the operation, e.g. `update.download`, `workflow:<id>`. */
  id: string
  kind: "workflow" | "update" | "import" | "export" | "migration" | "device"
  title: string
  status: OperationStatus
  /** Percent, when the operation reports it. */
  progress?: number
  completed?: number
  total?: number
  startedAt: number
  finishedAt?: number
  /** Short, safe summary; never raw backend output. */
  error?: string
  /** Only for operations that can stop safely. */
  cancel?: () => void
}

/** How long a finished operation stays visible. */
export const FINISHED_VISIBLE_MS = 6000

export class OperationStore extends Store<Operation> {
  private now: () => number

  constructor(now: () => number = Date.now) {
    super()
    this.now = now
  }

  /** Starts (or updates) an operation; `startedAt` defaults to now. */
  start(operation: Omit<Operation, "status" | "startedAt"> & { startedAt?: number }) {
    const previous = this.get(operation.id)
    this.put(operation.id, {
      ...operation,
      status: "running",
      startedAt:
        operation.startedAt ?? (previous?.status === "running" ? previous.startedAt : this.now()),
    })
  }

  update(
    id: string,
    change: Partial<Pick<Operation, "progress" | "completed" | "total" | "title">>,
  ) {
    const current = this.get(id)
    if (current?.status === "running") this.put(id, { ...current, ...change })
  }

  finish(id: string, result: { error?: string; cancelled?: boolean } = {}) {
    const current = this.get(id)
    if (!current) return
    this.put(id, {
      ...current,
      status: result.cancelled ? "cancelled" : result.error ? "failed" : "succeeded",
      error: result.error,
      finishedAt: this.now(),
      cancel: undefined,
    })
  }

  /** Drops finished operations older than `FINISHED_VISIBLE_MS`. */
  prune() {
    const limit = this.now() - FINISHED_VISIBLE_MS
    for (const [id, operation] of this.values) {
      if (operation.finishedAt !== undefined && operation.finishedAt < limit) this.remove(id)
    }
  }

  /** Running operations first (oldest first), then the latest finished. */
  static ordered(operations: Operation[]) {
    return [...operations].sort((a, b) => {
      if ((a.status === "running") !== (b.status === "running")) {
        return a.status === "running" ? -1 : 1
      }
      return a.status === "running"
        ? a.startedAt - b.startedAt
        : (b.finishedAt ?? 0) - (a.finishedAt ?? 0)
    })
  }
}

export type Severity = "info" | "warning" | "error"

export interface AppNotification {
  /** Identifies the event; repeats of the same event are combined. */
  key: string
  severity: Severity
  title: string
  message?: string
  at: number
  /** How often the event happened while unread. */
  count: number
  read: boolean
}

/** Notifications kept; older ones are dropped. */
export const NOTIFICATION_LIMIT = 50

export class NotificationStore extends Store<AppNotification> {
  private now: () => number

  constructor(now: () => number = Date.now) {
    super()
    this.now = now
  }

  /**
   * Records an event. An unread notification with the same key is updated
   * instead of adding another, so a flapping connection does not spam.
   */
  notify(event: { key: string; severity: Severity; title: string; message?: string }) {
    const previous = this.get(event.key)
    const count = previous && !previous.read ? previous.count + 1 : 1
    // Re-insert so that the latest event is last.
    this.values.delete(event.key)
    this.put(event.key, { ...event, at: this.now(), count, read: false })
    while (this.values.size > NOTIFICATION_LIMIT) {
      const oldest = this.values.keys().next().value
      if (oldest === undefined) break
      this.values.delete(oldest)
    }
    this.publish()
  }

  markAllRead() {
    let changed = false
    for (const [key, value] of this.values) {
      if (!value.read) {
        this.values.set(key, { ...value, read: true })
        changed = true
      }
    }
    if (changed) this.publish()
  }

  clear() {
    if (this.values.size === 0) return
    this.values.clear()
    this.publish()
  }

  /** Newest first. */
  static newestFirst(notifications: AppNotification[]) {
    return [...notifications].reverse()
  }
}
