import { ChevronDown, Ellipsis, Loader2 } from "lucide-react"
import { Fragment } from "react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useTitlebarState } from "@/hooks/use-titlebar"
import { type WindowChrome, windowDragHandlers } from "@/hooks/use-window-chrome"
import {
  type TitlebarAction,
  type TitlebarConfig,
  type TitlebarStore,
  type TitlebarTone,
  arrangeActions,
} from "@/lib/titlebar"
import { cn } from "@/lib/utils"

/** One height and icon size for every control (tokens in index.css). */
export const CONTROL =
  "h-(--titlebar-control) gap-1.5 px-2.5 text-[0.8125rem] [&_svg:not([class*='size-'])]:size-(--titlebar-icon)"
export const CONTROL_ICON =
  "size-(--titlebar-control) [&_svg:not([class*='size-'])]:size-(--titlebar-icon)"

const TONES: Record<TitlebarTone, string> = {
  neutral: "bg-muted-foreground/50",
  positive: "bg-emerald-500",
  warning: "bg-amber-500",
}

/**
 * The toolbar of the main window: room for the native window controls, then
 * `leading` (the device and profile selectors, only for views that work on
 * them), the context of the current page — what it registered with
 * `useTitlebar` — and `trailing` (the app's own commands). It knows no page
 * or module itself. Without `leading` the page's context starts where the
 * selectors would, with no gap. On macOS it is the window's titlebar.
 */
export function AppTitlebar({
  store,
  chrome,
  fallbackTitle,
  leading,
  trailing,
  context,
}: {
  store: TitlebarStore
  chrome: WindowChrome
  /**
   * Shown until the page registers its own title, e.g. while it loads;
   * empty for pages without a title in the toolbar.
   */
  fallbackTitle: string
  /** Shown first (after the window controls), before the page's context. */
  leading?: React.ReactNode
  /** Rightmost elements: the app's commands (notifications, settings, …). */
  trailing?: React.ReactNode
  /** What the view on screen works on (`lib/toolbar-context.ts`), for styling and tests. */
  context?: string
}) {
  const config = useTitlebarState(store)
  // The latest callbacks, also when only they changed.
  const select = (action: TitlebarAction) => {
    store.action(action.id)?.onSelect?.()
  }
  return (
    <header
      data-slot="titlebar"
      data-context={context}
      aria-label="Toolbar"
      className="relative flex h-(--titlebar-height) shrink-0 items-stretch border-b bg-background select-none"
      style={{ "--titlebar-controls-inset": `${chrome.controlsInset}px` } as React.CSSProperties}
      {...windowDragHandlers(chrome)}
    >
      {/*
        Room for the native window controls (macOS traffic lights; none in
        fullscreen and on Windows and Linux, where the window manager draws
        them above the toolbar). Nothing else: the app is navigated with the
        rail, the selectors and the commands at the right.
      */}
      <div
        aria-hidden
        data-slot="titlebar-controls-inset"
        className="w-(--titlebar-controls-inset) shrink-0"
      />
      {leading && (
        <div
          data-slot="titlebar-leading"
          className="flex shrink-0 items-center gap-(--titlebar-gap) pl-(--titlebar-padding-x)"
        >
          {leading}
        </div>
      )}
      <div className="@container flex min-w-0 flex-1 items-center gap-2 px-(--titlebar-padding-x)">
        <Heading config={config} fallbackTitle={fallbackTitle} />
        {config.status && (
          <span
            className="hidden h-(--titlebar-control) max-w-48 shrink-0 items-center gap-1.5 rounded-full border px-2.5 text-xs leading-none @xl:flex"
            title={config.status.hint}
          >
            <span
              className={cn("size-1.5 rounded-full", TONES[config.status.tone ?? "neutral"])}
              aria-hidden
            />
            {config.status.caption && (
              <span className="text-muted-foreground">{config.status.caption}</span>
            )}
            <span className="truncate font-medium">{config.status.label}</span>
          </span>
        )}
        <Actions actions={config.actions} select={select} />
      </div>
      {trailing && (
        <div
          data-slot="titlebar-trailing"
          className="flex shrink-0 items-center gap-(--titlebar-gap) pr-(--titlebar-padding-x)"
        >
          {trailing}
        </div>
      )}
      {config.progress !== undefined && <ProgressLine progress={config.progress} />}
    </header>
  )
}

/**
 * The page's location and title. Without a title (the device editor, whose
 * context is the toolbar's selectors; a settings page, which shows its own
 * title) only the breadcrumbs are shown, and without those nothing: the
 * space goes to the page's actions.
 */
function Heading({ config, fallbackTitle }: { config: TitlebarConfig; fallbackTitle: string }) {
  const title = config.title ?? fallbackTitle
  const crumbs = config.breadcrumbs ?? []
  return (
    <div data-slot="titlebar-heading" className="flex min-w-0 flex-1 items-center gap-1.5">
      {crumbs.length > 0 && (
        <ol
          aria-label="Location"
          className={cn(
            "shrink-0 items-center gap-1.5 text-sm leading-5",
            // Without a title the location is the heading: always shown.
            title ? "hidden @lg:flex" : "flex",
          )}
        >
          {crumbs.map((crumb, index) => (
            <Fragment key={crumb.label}>
              {index > 0 && (
                <li aria-hidden className="text-muted-foreground/60">
                  /
                </li>
              )}
              <li>
                {crumb.onSelect ? (
                  <button
                    type="button"
                    className="rounded-sm text-muted-foreground hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    onClick={crumb.onSelect}
                  >
                    {crumb.label}
                  </button>
                ) : (
                  <span className="text-muted-foreground">{crumb.label}</span>
                )}
              </li>
            </Fragment>
          ))}
          {title && (
            <li aria-hidden className="text-muted-foreground/60">
              /
            </li>
          )}
        </ol>
      )}
      {title && (
        <div className="min-w-0">
          <h1 className="truncate text-sm leading-5 font-semibold" title={title}>
            {title}
          </h1>
          {config.subtitle && (
            <p className="hidden truncate text-xs leading-4 text-muted-foreground @3xl:block">
              {config.subtitle}
            </p>
          )}
        </div>
      )}
    </div>
  )
}

function Actions({
  actions,
  select,
}: {
  actions: TitlebarAction[] | undefined
  select: (action: TitlebarAction) => void
}) {
  const { primary, secondary, overflow } = arrangeActions(actions)
  if (primary.length + secondary.length + overflow.length === 0) return null
  return (
    <div
      role="toolbar"
      aria-label="Page actions"
      className="flex shrink-0 items-center gap-(--titlebar-gap)"
    >
      {secondary.map((action) => (
        <ActionButton
          key={action.id}
          action={action}
          select={select}
          variant="ghost"
          className="hidden @2xl:inline-flex"
        />
      ))}
      {primary.map((action) => (
        <ActionButton key={action.id} action={action} select={select} variant="default" />
      ))}
      {(overflow.length > 0 || secondary.length > 0) && (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className={cn(CONTROL_ICON, overflow.length === 0 && "@2xl:hidden")}
              aria-label="More actions"
              title="More actions"
            >
              <Ellipsis />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-48">
            {secondary.map((action) => (
              <MenuAction key={action.id} action={action} select={select} className="@2xl:hidden" />
            ))}
            {secondary.length > 0 && overflow.length > 0 && (
              <DropdownMenuSeparator className="@2xl:hidden" />
            )}
            {overflow.map((action) => (
              <MenuAction key={action.id} action={action} select={select} />
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </div>
  )
}

function ActionButton({
  action,
  select,
  variant,
  className,
}: {
  action: TitlebarAction
  select: (action: TitlebarAction) => void
  variant: "default" | "ghost"
  className?: string
}) {
  const Icon = action.busy ? Loader2 : action.icon
  const content = (
    <>
      {Icon && <Icon className={cn(action.busy && "animate-spin")} />}
      <span className={cn(Icon && variant === "default" && "hidden @md:inline")}>
        {action.label}
      </span>
    </>
  )
  const common = {
    variant,
    size: "sm" as const,
    className: cn(CONTROL, className),
    disabled: action.disabled,
    title: action.hint ?? action.label,
    "aria-label": action.label,
  }
  if (action.items) {
    return (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button {...common}>
            {content}
            <ChevronDown className="opacity-60" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-48">
          <DropdownMenuLabel>{action.label}</DropdownMenuLabel>
          <DropdownMenuSeparator />
          <MenuItems action={action} />
        </DropdownMenuContent>
      </DropdownMenu>
    )
  }
  return (
    <Button
      {...common}
      onClick={() => {
        select(action)
      }}
    >
      {content}
    </Button>
  )
}

function MenuItems({ action }: { action: TitlebarAction }) {
  return (
    <>
      {action.items?.map((item) => (
        <DropdownMenuCheckboxItem
          key={item.id}
          checked={item.checked ?? false}
          onSelect={item.onSelect}
        >
          {item.label}
        </DropdownMenuCheckboxItem>
      ))}
    </>
  )
}

function MenuAction({
  action,
  select,
  className,
}: {
  action: TitlebarAction
  select: (action: TitlebarAction) => void
  className?: string
}) {
  const Icon = action.icon
  if (action.items) {
    return (
      <DropdownMenuSub>
        <DropdownMenuSubTrigger className={className} disabled={action.disabled}>
          {Icon && <Icon />}
          {action.label}
        </DropdownMenuSubTrigger>
        <DropdownMenuSubContent>
          <MenuItems action={action} />
        </DropdownMenuSubContent>
      </DropdownMenuSub>
    )
  }
  return (
    <DropdownMenuItem
      className={className}
      disabled={action.disabled}
      title={action.hint}
      onSelect={() => {
        select(action)
      }}
    >
      {Icon && <Icon />}
      {action.label}
    </DropdownMenuItem>
  )
}

function ProgressLine({ progress }: { progress: boolean | number }) {
  const percent = typeof progress === "number" ? Math.max(0, Math.min(100, progress)) : null
  return (
    <div
      role="progressbar"
      aria-label="Progress"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
      className="pointer-events-none absolute inset-x-0 -bottom-px h-0.5 overflow-hidden"
    >
      <div
        className={cn(
          "h-full bg-primary",
          percent === null && "w-1/3 animate-[titlebar-progress_1.2s_ease-in-out_infinite]",
        )}
        style={percent === null ? undefined : { width: `${percent}%` }}
      />
    </div>
  )
}
