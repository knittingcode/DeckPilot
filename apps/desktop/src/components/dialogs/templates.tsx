import {
  CircleAlert,
  Info,
  Loader2,
  type LucideIcon,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react"
import { Slot } from "radix-ui"
import { type ReactNode, useId, useRef, useState } from "react"

import { type ModalSize, ModalShell } from "@/components/modal-shell"
import { Input } from "@/components/ui/input"
import { Progress } from "@/components/ui/progress"
import { useTranslation } from "@/i18n"
import { errorMessage } from "@/lib/api"
import { cn } from "@/lib/utils"

/**
 * The dialog templates of DeckPilot. Each is a thin, typed arrangement of
 * `ModalShell` (presentation "dialog"): same header, body, inline error and
 * footer, same tokens in light and dark. Use them directly as components,
 * or imperatively through `useDialogs()` (dialog-service.tsx). See
 * docs/dialogs.md.
 *
 * Shared rules:
 * - An async action keeps the dialog open while it runs: the primary
 *   button shows its busy label, the other actions are disabled, and a
 *   second submit is ignored. On success the dialog closes; on failure the
 *   error is shown inline and the dialog stays open.
 * - Errors are user-facing text. `describeError` turns a thrown value
 *   into it (default: the backend's message, which DeckPilot's commands
 *   word for users).
 * - Escape, the close button and a click outside cancel, unless an action
 *   is running.
 */

/** An action; a returned promise keeps the dialog busy until it settles. */
type Action = () => unknown

/** Runs `action` with busy state and inline error; resolves `true` on success. */
function useAction(describeError: (error: unknown) => string = errorMessage) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const running = useRef(false)
  const run = async (action: Action): Promise<boolean> => {
    if (running.current) return false
    running.current = true
    setBusy(true)
    setError(null)
    try {
      await action()
      return true
    } catch (e) {
      setError(describeError(e))
      return false
    } finally {
      running.current = false
      setBusy(false)
    }
  }
  return { busy, error, setError, run }
}

/** Open state that works controlled (`open`) or from a `trigger`. */
function useOpenState(open: boolean | undefined, onOpenChange?: (open: boolean) => void) {
  const [own, setOwn] = useState(false)
  const controlled = open !== undefined
  return {
    open: controlled ? open : own,
    setOpen: (next: boolean) => {
      if (!controlled) setOwn(next)
      onOpenChange?.(next)
    },
  }
}

function Trigger({ children, onOpen }: { children?: ReactNode; onOpen: () => void }) {
  if (!children) return null
  return <Slot.Root onClick={onOpen}>{children}</Slot.Root>
}

export interface ConfirmDialogProps {
  /** Opens the dialog when clicked (uncontrolled). */
  trigger?: ReactNode
  open?: boolean
  onOpenChange?: (open: boolean) => void
  title: string
  description: ReactNode
  /** Further content between the description and the footer. */
  children?: ReactNode
  confirmLabel?: string
  busyLabel?: string
  cancelLabel?: string
  /** Deletes or replaces data: destructive button and icon. Default true. */
  destructive?: boolean
  icon?: LucideIcon
  /** May return a promise: the dialog stays open, busy, until it settles. */
  onConfirm: Action
  describeError?: (error: unknown) => string
}

/**
 * Asks before an action. With `destructive` (the default, for the many
 * delete confirmations) it is the Destructive template.
 */
export function ConfirmDialog({
  trigger,
  open,
  onOpenChange,
  title,
  description,
  children,
  confirmLabel,
  busyLabel,
  cancelLabel,
  destructive = true,
  icon,
  onConfirm,
  describeError,
}: ConfirmDialogProps) {
  const { t } = useTranslation()
  const state = useOpenState(open, onOpenChange)
  const { busy, error, setError, run } = useAction(describeError)
  const close = () => {
    if (busy) return
    setError(null)
    state.setOpen(false)
  }
  return (
    <>
      <Trigger
        onOpen={() => {
          state.setOpen(true)
        }}
      >
        {trigger}
      </Trigger>
      {state.open && (
        <ModalShell
          presentation="dialog"
          size="compact"
          icon={icon ?? (destructive ? TriangleAlert : undefined)}
          tone={destructive ? "destructive" : "default"}
          role="alertdialog"
          title={title}
          description={description}
          error={error}
          onClose={close}
          onSubmit={() => {
            void run(onConfirm).then((ok) => {
              if (ok) state.setOpen(false)
            })
          }}
          secondary={{
            label: cancelLabel ?? t("common:actions.cancel"),
            onClick: close,
            disabled: busy,
          }}
          primary={{
            label: confirmLabel ?? t("common:actions.delete"),
            loadingLabel: busyLabel,
            loading: busy,
            variant: destructive ? "destructive" : "default",
          }}
        >
          {children}
        </ModalShell>
      )}
    </>
  )
}

/** The Destructive template: `ConfirmDialog` that always deletes or replaces data. */
export function DestructiveDialog(props: Omit<ConfirmDialogProps, "destructive">) {
  return <ConfirmDialog {...props} destructive />
}

/** The Alert template: information, a warning or a recoverable error, one button. */
export function AlertDialog({
  open,
  onClose,
  tone = "info",
  title,
  description,
  children,
  closeLabel,
  onAcknowledge,
}: {
  open: boolean
  onClose: () => void
  tone?: "info" | "warning" | "error"
  title: string
  description?: ReactNode
  children?: ReactNode
  closeLabel?: string
  /**
   * The button (or Enter), when it must be told apart from Escape and the
   * close button, which call `onClose`. Default: `onClose`.
   */
  onAcknowledge?: () => void
}) {
  const { t } = useTranslation()
  if (!open) return null
  return (
    <ModalShell
      presentation="dialog"
      size="compact"
      icon={tone === "info" ? Info : tone === "warning" ? TriangleAlert : CircleAlert}
      tone={tone === "error" ? "destructive" : "default"}
      role="alertdialog"
      title={title}
      description={description}
      onClose={onClose}
      onSubmit={onAcknowledge ?? onClose}
      primary={{ label: closeLabel ?? t("common:actions.ok") }}
    >
      {children}
    </ModalShell>
  )
}

/**
 * The Form template: create or edit structured data. Fields are the
 * children (labelled controls with `FieldError` below each); `canSubmit`
 * gates the primary action. With `dirty`, closing asks before discarding.
 */
export function FormDialog({
  open,
  onClose,
  icon,
  title,
  description,
  children,
  submitLabel,
  busyLabel,
  cancelLabel,
  canSubmit = true,
  dirty = false,
  onSubmit,
  size = "medium",
  status,
  describeError,
}: {
  open: boolean
  onClose: () => void
  icon?: LucideIcon
  title: string
  description?: ReactNode
  children: ReactNode
  submitLabel: string
  busyLabel?: string
  cancelLabel?: string
  canSubmit?: boolean
  dirty?: boolean
  onSubmit: Action
  size?: ModalSize
  status?: ReactNode
  describeError?: (error: unknown) => string
}) {
  const { t } = useTranslation()
  const { busy, error, setError, run } = useAction(describeError)
  const [discarding, setDiscarding] = useState(false)
  if (!open) return null
  const requestClose = () => {
    if (busy) return
    if (dirty) {
      setDiscarding(true)
      return
    }
    setError(null)
    onClose()
  }
  return (
    <>
      <ModalShell
        presentation="dialog"
        size={size}
        icon={icon}
        title={title}
        description={description}
        error={error}
        status={status}
        onClose={requestClose}
        onSubmit={() => {
          void run(onSubmit).then((ok) => {
            if (ok) onClose()
          })
        }}
        secondary={{
          label: cancelLabel ?? t("common:actions.cancel"),
          onClick: requestClose,
          disabled: busy,
        }}
        primary={{
          label: submitLabel,
          loadingLabel: busyLabel,
          loading: busy,
          disabled: !canSubmit,
        }}
      >
        {children}
      </ModalShell>
      {/* Nested on purpose: it belongs to this form and returns to it. */}
      <ConfirmDialog
        open={discarding}
        onOpenChange={setDiscarding}
        title={t("dialogs:discard.title")}
        description={t("dialogs:discard.description")}
        confirmLabel={t("dialogs:discard.confirm")}
        onConfirm={() => {
          setDiscarding(false)
          setError(null)
          onClose()
        }}
      />
    </>
  )
}

export interface SelectionOption {
  id: string
  label: string
  description?: string
  disabled?: boolean
}

/**
 * The Selection template: pick one item (a layout, profile, page…). A
 * radio group, so arrow keys move and Enter confirms; a filter appears
 * when the list is long.
 */
export function SelectionDialog({
  open,
  onClose,
  icon,
  title,
  description,
  options,
  initial = null,
  submitLabel,
  busyLabel,
  cancelLabel,
  emptyText,
  onSelect,
  describeError,
  children,
}: {
  open: boolean
  onClose: () => void
  icon?: LucideIcon
  title: string
  description?: ReactNode
  options: SelectionOption[]
  initial?: string | null
  submitLabel?: string
  busyLabel?: string
  cancelLabel?: string
  emptyText?: string
  onSelect: (id: string) => unknown
  describeError?: (error: unknown) => string
  /** Shown above the options (e.g. who asks). */
  children?: ReactNode
}) {
  const { t } = useTranslation()
  const [selected, setSelected] = useState<string | null>(initial)
  const [filter, setFilter] = useState("")
  const { busy, error, run } = useAction(describeError)
  const name = useId()
  if (!open) return null
  const query = filter.trim().toLowerCase()
  const shown = query
    ? options.filter((o) => `${o.label} ${o.description ?? ""}`.toLowerCase().includes(query))
    : options
  return (
    <ModalShell
      presentation="dialog"
      size="compact"
      icon={icon}
      title={title}
      description={description}
      error={error}
      onClose={() => {
        if (!busy) onClose()
      }}
      onSubmit={() => {
        if (!selected) return
        void run(() => onSelect(selected)).then((ok) => {
          if (ok) onClose()
        })
      }}
      secondary={{
        label: cancelLabel ?? t("common:actions.cancel"),
        onClick: onClose,
        disabled: busy,
      }}
      primary={{
        label: submitLabel ?? t("dialogs:selection.choose"),
        loadingLabel: busyLabel,
        loading: busy,
        disabled: !selected,
      }}
    >
      {children}
      {options.length > 8 && (
        <Input
          type="search"
          aria-label={t("dialogs:selection.filter")}
          placeholder={t("dialogs:selection.filter")}
          value={filter}
          onChange={(e) => {
            setFilter(e.target.value)
          }}
        />
      )}
      {shown.length === 0 ? (
        <p className="text-sm text-muted-foreground">{emptyText ?? t("dialogs:selection.empty")}</p>
      ) : (
        <div
          role="radiogroup"
          aria-label={title}
          className="relative grid max-h-72 gap-1 overflow-y-auto"
        >
          {shown.map((option) => (
            <label
              key={option.id}
              className={cn(
                "flex cursor-pointer items-start gap-3 rounded-md border px-3 py-2 text-sm has-checked:border-primary has-checked:bg-accent has-disabled:cursor-not-allowed has-disabled:opacity-50",
              )}
            >
              <input
                type="radio"
                name={name}
                value={option.id}
                className="mt-0.5 accent-primary"
                checked={selected === option.id}
                disabled={option.disabled || busy}
                onChange={() => {
                  setSelected(option.id)
                }}
              />
              <span className="grid gap-0.5">
                <span className="font-medium">{option.label}</span>
                {option.description && (
                  <span className="text-xs text-muted-foreground">{option.description}</span>
                )}
              </span>
            </label>
          ))}
        </div>
      )}
    </ModalShell>
  )
}

/**
 * The Permission template: reviewing and granting capabilities to a
 * plugin. Host-owned — plugins can never open it. It always names who
 * asks (`requester`), which a plugin cannot change.
 */
export function PermissionDialog({
  open,
  onClose,
  requester,
  title,
  description,
  children,
  status,
  allowLabel,
  busyLabel,
  canAllow = true,
  onAllow,
  size = "medium",
  describeError,
}: {
  open: boolean
  onClose: () => void
  requester: { name: string; detail?: ReactNode }
  title: string
  description?: ReactNode
  children: ReactNode
  status?: ReactNode
  allowLabel?: string
  busyLabel?: string
  canAllow?: boolean
  onAllow: Action
  size?: ModalSize
  describeError?: (error: unknown) => string
}) {
  const { t } = useTranslation()
  const { busy, error, run } = useAction(describeError)
  if (!open) return null
  return (
    <ModalShell
      presentation="dialog"
      size={size}
      icon={ShieldCheck}
      title={title}
      description={description}
      error={error}
      status={status}
      onClose={() => {
        if (!busy) onClose()
      }}
      onSubmit={() => {
        void run(onAllow).then((ok) => {
          if (ok) onClose()
        })
      }}
      secondary={{ label: t("common:actions.cancel"), onClick: onClose, disabled: busy }}
      primary={{
        label: allowLabel ?? t("dialogs:permission.allow"),
        loadingLabel: busyLabel ?? t("common:status.working"),
        loading: busy,
        disabled: !canAllow,
      }}
    >
      <div
        className="flex items-baseline gap-2 text-sm"
        data-dialog-part="requester"
        aria-label={t("dialogs:permission.requestedBy")}
      >
        <span className="font-medium">{requester.name}</span>
        {requester.detail && <span className="text-muted-foreground">{requester.detail}</span>}
      </div>
      {children}
    </ModalShell>
  )
}

/**
 * The Progress template: a long-running operation. `value` 0–100, or
 * `null` while the amount is unknown. Without `onCancel` it cannot be
 * dismissed; it closes when the caller stops rendering it.
 */
export function ProgressDialog({
  open,
  title,
  description,
  value,
  status,
  onCancel,
  cancelLabel,
}: {
  open: boolean
  title: string
  description?: ReactNode
  value: number | null
  status?: ReactNode
  onCancel?: () => void
  cancelLabel?: string
}) {
  const { t } = useTranslation()
  if (!open) return null
  return (
    <ModalShell
      presentation="dialog"
      size="compact"
      blocking={!onCancel}
      title={title}
      description={description}
      onClose={onCancel}
      secondary={
        onCancel
          ? { label: cancelLabel ?? t("common:actions.cancel"), onClick: onCancel }
          : undefined
      }
    >
      <div className="grid gap-2" role="status" aria-live="polite">
        {value === null ? (
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" aria-hidden /> {t("common:status.working")}
          </p>
        ) : (
          <Progress value={value} aria-label={title} />
        )}
        {status && <p className="text-xs text-muted-foreground">{status}</p>}
      </div>
    </ModalShell>
  )
}
