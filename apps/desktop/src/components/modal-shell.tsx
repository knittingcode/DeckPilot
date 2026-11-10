import { Loader2, type LucideIcon } from "lucide-react"
import { type ReactNode, useEffect, useId, useRef } from "react"

import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { cn } from "@/lib/utils"

export interface ModalAction {
  label: string
  /** Shown instead of `label` while `loading`. */
  loadingLabel?: string
  onClick?: () => void
  disabled?: boolean
  loading?: boolean
  variant?: "default" | "outline" | "ghost" | "destructive"
}

/** Width behaviour of the shell; matches the native window size modes. */
export type ModalSize = "fit" | "compact" | "medium" | "large" | "fullscreen"

export interface ModalShellProps {
  /**
   * `window`: the content of a native window (the window is the modal).
   * `dialog`: an in-app dialog on top of the current page.
   */
  presentation?: "window" | "dialog"
  /** Dialog presentation only. */
  open?: boolean
  size?: ModalSize
  icon?: LucideIcon
  title: string
  description?: ReactNode
  children?: ReactNode
  /** With `onSubmit`, the primary action submits the form (Enter works). */
  primary?: ModalAction
  secondary?: ModalAction
  /** Replaces the generated footer. */
  footer?: ReactNode
  error?: string | null
  loading?: boolean
  /**
   * Must be completed: Escape and clicks outside do not close it and there
   * is no close button.
   */
  blocking?: boolean
  onClose?: () => void
  onSubmit?: () => void
  className?: string
}

const WIDTHS: Record<ModalSize, string> = {
  fit: "w-max min-w-[320px] max-w-[960px]",
  compact: "w-max min-w-[360px] max-w-[440px]",
  medium: "w-max min-w-[480px] max-w-[680px]",
  // Resizable windows fill whatever size the user gives them.
  large: "w-full min-h-svh",
  fullscreen: "w-full min-h-svh",
}

/**
 * The frame of every modal surface: header with optional icon, title and
 * description, a scrolling content area, an optional error, and a footer
 * with primary and secondary actions. It holds no application logic.
 */
export function ModalShell(props: ModalShellProps) {
  return props.presentation === "dialog" ? <DialogShell {...props} /> : <WindowShell {...props} />
}

function Actions({
  primary,
  secondary,
  submits,
}: Pick<ModalShellProps, "primary" | "secondary"> & { submits: boolean }) {
  if (!primary && !secondary) return null
  return (
    <>
      {secondary && (
        <Button
          type="button"
          variant={secondary.variant ?? "outline"}
          disabled={secondary.disabled}
          onClick={secondary.onClick}
        >
          {secondary.label}
        </Button>
      )}
      {primary && (
        <Button
          type={submits ? "submit" : "button"}
          variant={primary.variant ?? "default"}
          disabled={primary.disabled || primary.loading}
          onClick={submits ? undefined : primary.onClick}
        >
          {primary.loading && <Loader2 className="animate-spin" aria-hidden />}
          {primary.loading ? (primary.loadingLabel ?? primary.label) : primary.label}
        </Button>
      )}
    </>
  )
}

function Body({
  loading,
  error,
  children,
}: Pick<ModalShellProps, "loading" | "error" | "children">) {
  return (
    <>
      {error && (
        <Alert variant="destructive" role="alert">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
      {loading ? (
        <p className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
          <Loader2 className="size-4 animate-spin" aria-hidden /> Loading…
        </p>
      ) : (
        children
      )}
    </>
  )
}

/** Wraps `content` in a form when the shell submits. */
function MaybeForm({
  onSubmit,
  className,
  children,
}: {
  onSubmit?: () => void
  className: string
  children: ReactNode
}) {
  if (!onSubmit) return <div className={className}>{children}</div>
  return (
    <form
      className={className}
      noValidate
      onSubmit={(event) => {
        event.preventDefault()
        onSubmit()
      }}
    >
      {children}
    </form>
  )
}

function WindowShell({
  size = "fit",
  icon: Icon,
  title,
  description,
  children,
  primary,
  secondary,
  footer,
  error,
  loading,
  blocking = false,
  onClose,
  onSubmit,
  className,
}: ModalShellProps) {
  const titleId = useId()
  const descriptionId = useId()
  const root = useRef<HTMLElement>(null)

  // Focus the field marked autofocus, else the first control, once.
  useEffect(() => {
    const element = root.current
    if (!element || element.contains(document.activeElement)) return
    const target =
      element.querySelector<HTMLElement>("[autofocus], [data-autofocus]") ??
      element.querySelector<HTMLElement>("input, textarea, select, button:not([disabled])")
    target?.focus()
  }, [])

  // Escape closes non-blocking windows. The native window is the only
  // modal layer here, so there is no extra focus trap.
  useEffect(() => {
    if (blocking || !onClose) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) onClose()
    }
    window.addEventListener("keydown", onKey)
    return () => {
      window.removeEventListener("keydown", onKey)
    }
  }, [blocking, onClose])

  // The shell lays out at its natural size: nothing here depends on the
  // window's height, so measuring it cannot feed back into itself. When the
  // content is taller than the screen allows, the document scrolls and the
  // footer stays visible (sticky).
  return (
    <section
      ref={root}
      role={blocking ? "alertdialog" : "dialog"}
      aria-modal={blocking || undefined}
      aria-labelledby={titleId}
      aria-describedby={description ? descriptionId : undefined}
      data-modal-root=""
      className={cn("flex flex-col bg-background text-foreground", WIDTHS[size], className)}
    >
      <MaybeForm onSubmit={onSubmit} className="flex flex-1 flex-col">
        <header data-modal-part="header" className="flex items-start gap-3 px-6 pt-6 pb-4">
          {Icon && (
            <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground">
              <Icon className="size-4" aria-hidden />
            </div>
          )}
          <div className="grid min-w-0 gap-1">
            <h1 id={titleId} className="text-base leading-tight font-semibold">
              {title}
            </h1>
            {description && (
              <div id={descriptionId} className="text-sm text-muted-foreground">
                {description}
              </div>
            )}
          </div>
        </header>
        <div data-modal-part="body" className="grid flex-1 content-start gap-4 px-6 pb-2">
          <Body loading={loading} error={error}>
            {children}
          </Body>
        </div>
        {(footer ?? primary ?? secondary) && (
          <footer
            data-modal-part="footer"
            className="sticky bottom-0 flex flex-wrap items-center justify-end gap-2 border-t bg-background px-6 py-4"
          >
            {footer ?? (
              <Actions primary={primary} secondary={secondary} submits={Boolean(onSubmit)} />
            )}
          </footer>
        )}
        {!footer && !primary && !secondary && <div className="h-4 shrink-0" aria-hidden />}
      </MaybeForm>
    </section>
  )
}

function DialogShell({
  open = true,
  size = "medium",
  icon: Icon,
  title,
  description,
  children,
  primary,
  secondary,
  footer,
  error,
  loading,
  blocking = false,
  onClose,
  onSubmit,
  className,
}: ModalShellProps) {
  const prevent = (event: Event) => {
    if (blocking) event.preventDefault()
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && !blocking) onClose?.()
      }}
    >
      <DialogContent
        showCloseButton={!blocking}
        onEscapeKeyDown={prevent}
        onPointerDownOutside={prevent}
        onInteractOutside={prevent}
        className={cn(
          "flex max-h-[90vh] flex-col",
          size === "compact" && "sm:max-w-md",
          size === "large" && "sm:max-w-4xl",
          className,
        )}
      >
        <MaybeForm onSubmit={onSubmit} className="flex min-h-0 flex-col gap-4">
          <DialogHeader className="flex-row items-start gap-3 text-left">
            {Icon && (
              <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground">
                <Icon className="size-4" aria-hidden />
              </div>
            )}
            <div className="grid gap-1">
              <DialogTitle>{title}</DialogTitle>
              {description && <DialogDescription>{description}</DialogDescription>}
            </div>
          </DialogHeader>
          <div className="grid min-h-0 gap-4 overflow-y-auto">
            <Body loading={loading} error={error}>
              {children}
            </Body>
          </div>
          {(footer ?? primary ?? secondary) && (
            <DialogFooter>
              {footer ?? (
                <Actions primary={primary} secondary={secondary} submits={Boolean(onSubmit)} />
              )}
            </DialogFooter>
          )}
        </MaybeForm>
      </DialogContent>
    </Dialog>
  )
}
