import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react"

import { useTranslation } from "@/i18n"

import { DialogsContext, ServiceDialogOpenContext } from "./use-dialogs"
import { AlertDialog, ConfirmDialog, type SelectionOption, SelectionDialog } from "./templates"

/**
 * The dialog service: ask the user from code (`await dialogs.confirm(…)`)
 * instead of keeping open state in every component.
 *
 * Lifecycle rules (docs/dialogs.md):
 * - One service dialog is visible at a time. Requests made while one is
 *   open wait in a FIFO queue; nothing overlaps unpredictably.
 * - A dialog a component renders itself (a form, the authentication
 *   dialog) may open one service dialog on top: that is the only nesting,
 *   and Radix keeps focus inside the topmost dialog and returns it when it
 *   closes.
 * - Every request settles exactly once: confirmed (`true` / the choice),
 *   or cancelled (`false` / `null`) by Cancel, Escape, the close button, or
 *   when the provider unmounts (the window closes, the app locks). Nothing
 *   keeps waiting on a dialog that cannot be answered.
 * - `onConfirm` runs inside the dialog: busy state, inline error, no double
 *   submission; the request resolves `true` only after it succeeded.
 */

export interface ConfirmRequest {
  title: string
  description: ReactNode
  confirmLabel?: string
  busyLabel?: string
  cancelLabel?: string
  onConfirm?: () => unknown
  describeError?: (error: unknown) => string
}

export interface AlertRequest {
  title: string
  description?: ReactNode
  tone?: "info" | "warning" | "error"
  closeLabel?: string
}

export interface SelectRequest {
  title: string
  description?: ReactNode
  options: SelectionOption[]
  initial?: string | null
  submitLabel?: string
}

export interface Dialogs {
  /** A non-destructive confirmation; `true` when confirmed. */
  confirm: (request: ConfirmRequest) => Promise<boolean>
  /** A confirmation that deletes or replaces data; `true` when confirmed. */
  destructive: (request: ConfirmRequest) => Promise<boolean>
  alert: (request: AlertRequest) => Promise<void>
  /** The chosen option's id, or `null` when cancelled. */
  select: (request: SelectRequest) => Promise<string | null>
}

type Entry =
  | {
      id: number
      kind: "confirm" | "destructive"
      request: ConfirmRequest
      settle: (v: boolean) => void
    }
  | { id: number; kind: "alert"; request: AlertRequest; settle: () => void }
  | { id: number; kind: "select"; request: SelectRequest; settle: (v: string | null) => void }

/** Settles an entry as cancelled. */
function cancel(entry: Entry) {
  if (entry.kind === "alert") entry.settle()
  else if (entry.kind === "select") entry.settle(null)
  else entry.settle(false)
}

export function DialogProvider({ children }: { children: ReactNode }) {
  const [queue, setQueue] = useState<Entry[]>([])
  const pending = useRef<Entry[]>([])
  const next = useRef(1)
  useEffect(() => {
    pending.current = queue
  }, [queue])

  // Unmounting (window closed, app locked) cancels everything still open.
  useEffect(
    () => () => {
      for (const entry of pending.current) cancel(entry)
    },
    [],
  )

  const enqueue = useCallback(<T,>(make: (id: number, settle: (v: T) => void) => Entry) => {
    return new Promise<T>((resolve) => {
      const id = next.current++
      let settled = false
      const settle = (value: T) => {
        if (settled) return
        settled = true
        setQueue((q) => q.filter((e) => e.id !== id))
        resolve(value)
      }
      setQueue((q) => [...q, make(id, settle)])
    })
  }, [])

  const dialogs = useMemo<Dialogs>(
    () => ({
      confirm: (request) =>
        enqueue<boolean>((id, settle) => ({ id, kind: "confirm", request, settle })),
      destructive: (request) =>
        enqueue<boolean>((id, settle) => ({ id, kind: "destructive", request, settle })),
      alert: (request) =>
        enqueue<undefined>((id, settle) => ({
          id,
          kind: "alert",
          request,
          settle: () => {
            settle(undefined)
          },
        })),
      select: (request) =>
        enqueue<string | null>((id, settle) => ({ id, kind: "select", request, settle })),
    }),
    [enqueue],
  )

  const current = queue[0]
  return (
    <DialogsContext value={dialogs}>
      <ServiceDialogOpenContext value={current !== undefined}>
        {children}
        {current && <Host key={current.id} entry={current} />}
      </ServiceDialogOpenContext>
    </DialogsContext>
  )
}

function Host({ entry }: { entry: Entry }) {
  const { t } = useTranslation()
  switch (entry.kind) {
    case "confirm":
    case "destructive": {
      const { request } = entry
      return (
        <ConfirmDialog
          open
          onOpenChange={(open) => {
            if (!open) entry.settle(false)
          }}
          destructive={entry.kind === "destructive"}
          title={request.title}
          description={request.description}
          confirmLabel={
            request.confirmLabel ??
            (entry.kind === "destructive"
              ? t("common:actions.delete")
              : t("common:actions.continue"))
          }
          busyLabel={request.busyLabel}
          cancelLabel={request.cancelLabel}
          describeError={request.describeError}
          onConfirm={async () => {
            await request.onConfirm?.()
            entry.settle(true)
          }}
        />
      )
    }
    case "alert":
      return (
        <AlertDialog
          open
          onClose={entry.settle}
          title={entry.request.title}
          description={entry.request.description}
          tone={entry.request.tone}
          closeLabel={entry.request.closeLabel}
        />
      )
    case "select":
      return (
        <SelectionDialog
          open
          onClose={() => {
            entry.settle(null)
          }}
          title={entry.request.title}
          description={entry.request.description}
          options={entry.request.options}
          initial={entry.request.initial}
          submitLabel={entry.request.submitLabel}
          onSelect={(id) => {
            entry.settle(id)
          }}
        />
      )
  }
}
