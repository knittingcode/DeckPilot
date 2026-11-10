import { ChevronLeft, ChevronRight, ChevronDown, Ellipsis, Loader2 } from "lucide-react"
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
import { SidebarTrigger, useSidebar } from "@/components/ui/sidebar"
import { SHORTCUTS } from "@/hooks/use-navigation"
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
const CONTROL =
  "h-(--titlebar-control) gap-1.5 px-2.5 text-[0.8125rem] [&_svg:not([class*='size-'])]:size-(--titlebar-icon)"
const CONTROL_ICON =
  "size-(--titlebar-control) [&_svg:not([class*='size-'])]:size-(--titlebar-icon)"

const TONES: Record<TitlebarTone, string> = {
  neutral: "bg-muted-foreground/50",
  positive: "bg-emerald-500",
  warning: "bg-amber-500",
}

export interface TitlebarHistory {
  canGoBack: boolean
  canGoForward: boolean
  back: () => void
  forward: () => void
}

/**
 * The shared toolbar of the main window, in Workspace and Settings mode.
 * It shows what the current page registered (`useTitlebar`) and knows no
 * page or module itself. On macOS it is the window's titlebar.
 */
export function AppTitlebar({
  store,
  chrome,
  history,
  fallbackTitle,
  indicator,
}: {
  store: TitlebarStore
  chrome: WindowChrome
  history: TitlebarHistory
  /** Shown until the page registers its own title, e.g. while it loads. */
  fallbackTitle: string
  /** Rightmost element, e.g. the connection state. */
  indicator?: React.ReactNode
}) {
  const config = useTitlebarState(store)
  const { state, isMobile } = useSidebar()
  const aligned = state === "expanded" && !isMobile
  // The latest callbacks, also when only they changed.
  const select = (action: TitlebarAction) => {
    store.action(action.id)?.onSelect?.()
  }
  return (
    <header
      data-slot="titlebar"
      aria-label="Toolbar"
      className="relative flex h-(--titlebar-height) shrink-0 items-stretch border-b bg-background select-none"
      style={{ "--titlebar-controls-inset": `${chrome.controlsInset}px` } as React.CSSProperties}
      {...windowDragHandlers(chrome)}
    >
      {/*
        Fixed navigation region: the traffic-light safe area followed by the
        sidebar toggle and history. Its width depends only on tokens and the
        window's fullscreen state — never on the sidebar — so the controls
        stay put when the sidebar collapses or expands.
      */}
      <div
        data-slot="titlebar-navigation"
        className={cn(
          "flex w-(--titlebar-navigation-width) shrink-0 items-center gap-(--titlebar-gap) pr-(--titlebar-padding-x) pl-[calc(var(--titlebar-controls-inset)+var(--titlebar-padding-x))]",
          aligned && "bg-sidebar",
        )}
      >
        <SidebarTrigger className={CONTROL_ICON} title={`Toggle sidebar (${SHORTCUTS.sidebar})`} />
        <nav aria-label="History" className="flex items-center gap-(--titlebar-gap)">
          <Button
            variant="ghost"
            size="icon"
            className={CONTROL_ICON}
            aria-label="Back"
            title={`Back (${SHORTCUTS.back})`}
            disabled={!history.canGoBack}
            onClick={history.back}
          >
            <ChevronLeft />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className={CONTROL_ICON}
            aria-label="Forward"
            title={`Forward (${SHORTCUTS.forward})`}
            disabled={!history.canGoForward}
            onClick={history.forward}
          >
            <ChevronRight />
          </Button>
        </nav>
      </div>
      {/* Continues the expanded sidebar (background and divider) up to its edge. */}
      <div
        aria-hidden
        data-slot="titlebar-sidebar-extension"
        className={cn(
          "shrink-0 transition-[width] duration-200 ease-linear",
          aligned
            ? "w-[max(0px,calc(var(--sidebar-width)-var(--titlebar-navigation-width)))] border-r bg-sidebar"
            : "w-0",
        )}
      />
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
        {indicator}
      </div>
      {config.progress !== undefined && <ProgressLine progress={config.progress} />}
    </header>
  )
}

function Heading({ config, fallbackTitle }: { config: TitlebarConfig; fallbackTitle: string }) {
  const title = config.title ?? fallbackTitle
  return (
    <div className="flex min-w-0 flex-1 items-center gap-1.5">
      {config.breadcrumbs && config.breadcrumbs.length > 0 && (
        <ol
          aria-label="Location"
          className="hidden shrink-0 items-center gap-1.5 text-sm leading-5 @lg:flex"
        >
          {config.breadcrumbs.map((crumb) => (
            <Fragment key={crumb.label}>
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
              <li aria-hidden className="text-muted-foreground/60">
                /
              </li>
            </Fragment>
          ))}
        </ol>
      )}
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
