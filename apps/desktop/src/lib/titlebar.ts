import type { LucideIcon } from "lucide-react"

/**
 * What the shared titlebar shows for the current page. Pages, editors inside
 * them and module pages contribute parts of it declaratively (see
 * `useTitlebar`); the titlebar itself knows no page or module.
 */

export interface TitlebarMenuItem {
  id: string
  label: string
  /** Marks the current choice. */
  checked?: boolean
  onSelect: () => void
}

export type TitlebarPlacement = "primary" | "secondary" | "overflow"

export interface TitlebarAction {
  /** Stable within the page, e.g. `workflows.run`. */
  id: string
  label: string
  icon?: LucideIcon
  /**
   * `primary`: the page's main command, always visible. `secondary`: shown
   * while there is room, otherwise in the overflow menu. `overflow`: always
   * in the overflow menu. Default: `secondary`.
   */
  placement?: TitlebarPlacement
  /** Ordering within a placement; lower first. */
  order?: number
  disabled?: boolean
  /** Why the action is disabled, or more detail. */
  hint?: string
  busy?: boolean
  /** A command… */
  onSelect?: () => void
  /** …or a menu of choices. */
  items?: TitlebarMenuItem[]
}

export interface TitlebarBreadcrumb {
  label: string
  onSelect?: () => void
}

export type TitlebarTone = "neutral" | "positive" | "warning"

export interface TitlebarStatus {
  label: string
  /** Shown before the label in smaller type, e.g. "Profile". */
  caption?: string
  tone?: TitlebarTone
  hint?: string
}

export interface TitlebarConfig {
  title?: string
  subtitle?: string
  breadcrumbs?: TitlebarBreadcrumb[]
  actions?: TitlebarAction[]
  status?: TitlebarStatus
  /** `true`: indeterminate progress; a number: percent. */
  progress?: boolean | number
}

/**
 * Merges the contributions of several components, in registration order.
 * Single values: the last contribution that sets one wins. Actions are
 * combined; a later action with the same id replaces an earlier one.
 */
export function mergeTitlebar(contributions: TitlebarConfig[]): TitlebarConfig {
  const merged: TitlebarConfig = {}
  const actions = new Map<string, TitlebarAction>()
  for (const config of contributions) {
    if (config.title !== undefined) merged.title = config.title
    if (config.subtitle !== undefined) merged.subtitle = config.subtitle
    if (config.breadcrumbs !== undefined) merged.breadcrumbs = config.breadcrumbs
    if (config.status !== undefined) merged.status = config.status
    if (config.progress !== undefined && config.progress !== false) {
      merged.progress = config.progress
    }
    for (const action of config.actions ?? []) actions.set(action.id, action)
  }
  if (actions.size > 0) merged.actions = [...actions.values()]
  return merged
}

/** Actions by placement, each sorted by `order` and then as registered. */
export function arrangeActions(actions: TitlebarAction[] = []) {
  const by = (placement: TitlebarPlacement) =>
    actions
      .map((action, index) => ({ action, index }))
      .filter(({ action }) => (action.placement ?? "secondary") === placement)
      .sort((a, b) => (a.action.order ?? 0) - (b.action.order ?? 0) || a.index - b.index)
      .map(({ action }) => action)
  return { primary: by("primary"), secondary: by("secondary"), overflow: by("overflow") }
}

/** What a configuration looks like, without its callbacks. */
function appearance(config: TitlebarConfig) {
  return JSON.stringify(config, (_, value: unknown) => {
    if (typeof value === "function") return undefined
    // Icon components: compared by name.
    if (value && typeof value === "object" && "$$typeof" in value) {
      return (value as { displayName?: string }).displayName ?? "icon"
    }
    return value
  })
}

type Listener = () => void

/**
 * Contributions of mounted components. Components register in effects and
 * do not subscribe, so registering never re-renders them; only the
 * titlebar subscribes. A contribution whose appearance did not change is
 * updated silently: the titlebar calls the latest callbacks through
 * `action()`.
 */
export class TitlebarStore {
  private contributions = new Map<string, TitlebarConfig>()
  private listeners = new Set<Listener>()
  private snapshot: TitlebarConfig = {}

  set(id: string, config: TitlebarConfig) {
    const previous = this.contributions.get(id)
    this.contributions.set(id, config)
    if (!previous || appearance(previous) !== appearance(config)) this.publish()
  }

  remove(id: string) {
    if (this.contributions.delete(id)) this.publish()
  }

  subscribe = (listener: Listener) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  getSnapshot = () => this.snapshot

  /** The current version of an action, with its latest callbacks. */
  action(id: string): TitlebarAction | undefined {
    let found: TitlebarAction | undefined
    for (const config of this.contributions.values()) {
      found = config.actions?.find((a) => a.id === id) ?? found
    }
    return found
  }

  private publish() {
    this.snapshot = mergeTitlebar([...this.contributions.values()])
    for (const listener of this.listeners) listener()
  }
}
