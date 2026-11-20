import { Loader2, type LucideIcon } from "lucide-react"
import { type ElementType, type ReactNode, useEffect, useId, useRef, useState } from "react"

import { FieldError } from "@/components/field-error"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog"
import { useTranslation } from "@/i18n"
import { cn } from "@/lib/utils"

/**
 * The frame of every modal surface in DeckPilot — native windows that act
 * as dialogs (`presentation="window"`) and dialogs inside a window
 * (`presentation="dialog"`). Both render the same parts with the same
 * tokens: header (optional icon tile, title, description), body, an inline
 * error, and a footer with the actions. The templates in
 * `components/dialogs` (confirm, destructive, form, selection, permission,
 * progress, credential, alert) are built on it; see docs/dialogs.md.
 *
 * It holds no application logic.
 */

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
  /** `destructive` colours the icon tile for actions that delete or replace data. */
  tone?: "default" | "destructive"
  title: string
  description?: ReactNode
  children?: ReactNode
  /** With `onSubmit`, the primary action submits the form (Enter works). */
  primary?: ModalAction
  secondary?: ModalAction
  /** Replaces the generated actions (the footer frame stays). */
  footer?: ReactNode
  /** Short text at the left of the footer, e.g. what the primary action will do. */
  status?: ReactNode
  /**
   * An error not tied to one field, shown inline at the end of the body
   * (icon and text, no box). Field errors belong below their field
   * (`FieldError`). Pass user-facing text, not raw backend errors.
   */
  error?: string | null
  loading?: boolean
  /**
   * Must be completed: Escape and clicks outside do not close it and there
   * is no close button.
   */
  blocking?: boolean
  /**
   * `alertdialog` for confirmations and alerts that interrupt the user:
   * announced as such, and a click outside does not dismiss it (Escape and
   * Cancel do). Blocking shells are always `alertdialog`.
   */
  role?: "dialog" | "alertdialog"
  onClose?: () => void
  onSubmit?: () => void
  className?: string
  /**
   * `compact`: a smaller icon centred on the title and description, and
   * tighter spacing, for short prompts such as a password check.
   */
  density?: "default" | "compact"
}

const WINDOW_WIDTHS: Record<ModalSize, string> = {
  fit: "w-max min-w-[320px] max-w-[960px]",
  compact: "w-max min-w-[360px] max-w-[440px]",
  medium: "w-max min-w-[480px] max-w-[680px]",
  // Resizable windows fill whatever size the user gives them.
  large: "w-full min-h-svh",
  fullscreen: "w-full min-h-svh",
}

const DIALOG_WIDTHS: Record<ModalSize, string> = {
  fit: "sm:max-w-fit",
  compact: "sm:max-w-md",
  medium: "sm:max-w-lg",
  large: "sm:max-w-3xl",
  fullscreen: "sm:max-w-[calc(100vw-4rem)]",
}

/**
 * The icon tile of a modal's header; also used where a screen stands in
 * for a modal (the main window's locked state), so both look the same.
 */
export function ModalIcon({
  icon: Icon,
  tone = "default",
}: {
  icon: LucideIcon
  /** Accepted for existing callers; every header uses the same icon size. */
  compact?: boolean
  tone?: "default" | "destructive"
}) {
  return (
    <div
      className={cn(
        "flex shrink-0 items-center justify-center",
        tone === "destructive" ? "bg-destructive text-white" : "bg-primary text-primary-foreground",
        "size-(--modal-icon) rounded-xl",
      )}
    >
      <Icon className="size-6" aria-hidden />
    </div>
  )
}

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
          aria-busy={primary.loading || undefined}
          onClick={submits ? undefined : primary.onClick}
        >
          {primary.loading && <Loader2 className="animate-spin" aria-hidden />}
          {primary.loading ? (primary.loadingLabel ?? primary.label) : primary.label}
        </Button>
      )}
    </>
  )
}

/** Wraps the shell in a form when it submits; never submits twice. */
function MaybeForm({
  onSubmit,
  primary,
  className,
  children,
}: {
  onSubmit?: () => void
  primary?: ModalAction
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
        // Enter in a field submits even while the button is disabled or
        // busy; the shell ignores it then, so nothing runs twice.
        if (primary?.loading || primary?.disabled) return
        onSubmit()
      }}
    >
      {children}
    </form>
  )
}

/** Header, body and footer: identical in windows and dialogs. */
function Parts({
  icon,
  tone,
  title,
  description,
  children,
  primary,
  secondary,
  footer,
  status,
  error,
  loading,
  onSubmit,
  compact,
  titleId,
  descriptionId,
  Title,
  Description,
  scrollBody,
}: ModalShellProps & {
  compact: boolean
  titleId: string
  descriptionId: string
  Title: ElementType
  Description: ElementType
  scrollBody: boolean
}) {
  const { t } = useTranslation()
  const errorId = useId()
  const hasFooter = Boolean(footer ?? primary ?? secondary)
  return (
    <>
      {/*
        One header for every dialog and density: a 48 px icon tile and the
        text group beside it. A title and a one-line description (24 + 2 +
        20 px) fit inside the tile's height; the group is at least as tall as
        the tile and centres its lines in it, so the two read as one unit.
        Longer text wraps and the header grows downward, the tile staying at
        the top: nothing is clipped or truncated (docs/dialogs.md).
      */}
      <header
        data-modal-part="header"
        className={cn("flex shrink-0 items-start gap-3 px-6 pb-5", compact ? "pt-5" : "pt-6")}
      >
        {icon && <ModalIcon icon={icon} tone={tone} />}
        <div
          data-modal-part="heading"
          className={cn(
            "flex min-w-0 flex-col justify-center gap-0.5",
            icon && "min-h-(--modal-icon)",
          )}
        >
          <Title
            {...(titleId ? { id: titleId } : {})}
            className="text-lg leading-6 font-semibold tracking-tight text-balance"
          >
            {title}
          </Title>
          {description && (
            <Description
              {...(descriptionId ? { id: descriptionId } : {})}
              className="text-sm leading-5 text-pretty text-muted-foreground"
            >
              {description}
            </Description>
          )}
        </div>
      </header>
      <div
        data-modal-part="body"
        className={cn(
          "grid flex-1 content-start gap-4 px-6",
          compact ? "pb-2" : "pb-4",
          !hasFooter && "pb-6",
          scrollBody && "min-h-0 overflow-y-auto",
        )}
      >
        {loading ? (
          <p className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
            <Loader2 className="size-4 animate-spin" aria-hidden /> {t("common:status.loading")}
          </p>
        ) : (
          children
        )}
        {error && <FieldError id={errorId} message={error} />}
      </div>
      {hasFooter && (
        <footer
          data-modal-part="footer"
          className={cn(
            "sticky bottom-0 flex shrink-0 flex-wrap items-center justify-end gap-2 border-t bg-background px-6",
            compact ? "py-3" : "py-4",
          )}
        >
          {status && (
            <div className="mr-auto min-w-0 text-xs text-muted-foreground" data-modal-part="status">
              {status}
            </div>
          )}
          {footer ?? (
            <Actions primary={primary} secondary={secondary} submits={Boolean(onSubmit)} />
          )}
        </footer>
      )}
    </>
  )
}

function WindowShell(props: ModalShellProps) {
  const { size = "fit", blocking = false, onClose, onSubmit, primary, className } = props
  const compact = props.density === "compact"
  const titleId = useId()
  const descriptionId = useId()
  const root = useRef<HTMLElement>(null)

  // Focus the field marked autofocus, else the first text field, else the
  // dialog itself — never a button: a window that appears while the user
  // is typing elsewhere must not let a stray Return trigger an action such
  // as a Touch ID prompt.
  useEffect(() => {
    const element = root.current
    if (!element || element.contains(document.activeElement)) return
    const target =
      element.querySelector<HTMLElement>("[autofocus], [data-autofocus]") ??
      element.querySelector<HTMLElement>("input, textarea, select") ??
      element
    target.focus()
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
      role={blocking ? "alertdialog" : (props.role ?? "dialog")}
      aria-modal={blocking || undefined}
      aria-labelledby={titleId}
      aria-describedby={props.description ? descriptionId : undefined}
      data-modal-root=""
      tabIndex={-1}
      className={cn(
        "flex flex-col bg-background text-foreground outline-none",
        WINDOW_WIDTHS[size],
        className,
      )}
    >
      <MaybeForm onSubmit={onSubmit} primary={primary} className="flex flex-1 flex-col">
        <Parts
          {...props}
          compact={compact}
          titleId={titleId}
          descriptionId={descriptionId}
          Title="h1"
          Description="div"
          scrollBody={false}
        />
      </MaybeForm>
    </section>
  )
}

function DialogShell(props: ModalShellProps) {
  const {
    open = true,
    size = "medium",
    blocking = false,
    onClose,
    onSubmit,
    primary,
    className,
  } = props
  const role = blocking ? "alertdialog" : (props.role ?? "dialog")
  // Radix returns the focus to a `DialogTrigger`, which these dialogs do
  // not use; without one the focus would be lost. Return it to whatever
  // had it when the dialog opened (the button, menu item or field).
  const [returnFocus] = useState(() =>
    document.activeElement instanceof HTMLElement ? document.activeElement : null,
  )
  const prevent = (event: Event) => {
    if (blocking) event.preventDefault()
  }
  const preventOutside = (event: Event) => {
    if (role === "alertdialog") event.preventDefault()
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && !blocking) onClose?.()
      }}
    >
      <DialogContent
        role={role}
        showCloseButton={!blocking}
        onCloseAutoFocus={(event) => {
          event.preventDefault()
          if (returnFocus?.isConnected) returnFocus.focus()
        }}
        onEscapeKeyDown={prevent}
        onPointerDownOutside={preventOutside}
        onInteractOutside={preventOutside}
        // Radix links its own title and description; without a description
        // the attribute is left out explicitly.
        {...(props.description ? {} : { "aria-describedby": undefined })}
        data-modal-root=""
        className={cn(
          "flex max-h-[90vh] flex-col gap-0 overflow-hidden p-0",
          DIALOG_WIDTHS[size],
          className,
        )}
      >
        <MaybeForm onSubmit={onSubmit} primary={primary} className="flex min-h-0 flex-col">
          <Parts
            {...props}
            compact={props.density === "compact"}
            // Radix generates and links the ids of its title and description.
            titleId=""
            descriptionId=""
            Title={DialogTitle}
            Description={props.description ? DialogDescription : "div"}
            scrollBody
          />
        </MaybeForm>
      </DialogContent>
    </Dialog>
  )
}
