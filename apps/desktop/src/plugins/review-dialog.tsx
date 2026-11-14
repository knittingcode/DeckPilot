import { Check, ShieldAlert } from "lucide-react"
import { useState } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"

import { pluginErrorText } from "./api"
import { PermissionList, PublisherLine } from "./components"
import { FIXTURE_LABEL } from "./fixtures"
import type { Grant } from "./permissions"
import type { RegistryPublisher } from "./types"

/**
 * - `new`: needs the user's consent now.
 * - `granted`: consent is stored; shown for context.
 * - `revoked`: the user revoked it; it stays revoked unless they allow it here.
 */
export type ReviewState = "new" | "granted" | "revoked"

export interface ReviewRequest {
  mode: "install" | "update" | "enable"
  pluginId: string
  name: string
  namespace: string
  publisher: RegistryPublisher | null
  version: string
  items: { grant: Grant; state: ReviewState }[]
  /** Permissions an update drops; shown so the change is complete. */
  removed?: string[]
  /** Resolves when done; a rejection is shown in the dialog. */
  confirm: (consent: string[]) => Promise<void>
}

const CONFIRM = { install: "Install", update: "Update", enable: "Enable" } as const

/**
 * Asks for consent before an install, an update that asks for more, or
 * enabling a plugin. Every new permission needs its own check; "Allow all"
 * is a separate, explicit action. The dialog keeps focus until closed.
 */
export function PermissionReviewDialog({
  request,
  fixtures,
  onClose,
}: {
  request: ReviewRequest
  fixtures: boolean
  onClose: () => void
}) {
  const [checked, setChecked] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<{ message: string; hint?: string } | null>(null)
  const required = request.items.filter((i) => i.state === "new").map((i) => i.grant.key)
  const missing = required.filter((key) => !checked.has(key)).length
  const states = new Map(request.items.map((i) => [i.grant.key, i.state]))

  const toggle = (key: string, value: boolean) => {
    setChecked((previous) => {
      const next = new Set(previous)
      if (value) next.add(key)
      else next.delete(key)
      return next
    })
  }

  const submit = () => {
    setBusy(true)
    setError(null)
    request.confirm([...checked]).then(onClose, (e: unknown) => {
      setError(pluginErrorText(e))
      setBusy(false)
    })
  }

  const title =
    request.mode === "install"
      ? `Install ${request.name} ${request.version}?`
      : request.mode === "update"
        ? `Update ${request.name} to ${request.version}?`
        : `Enable ${request.name}?`
  const intro =
    request.mode === "install"
      ? "Review what this plugin may do. Check each permission to consent to it; you can revoke permissions later in Installed."
      : request.mode === "update"
        ? "This version asks for permissions you have not granted. Check each new one to consent, or cancel to keep the current version."
        : "Review what the plugin may do once it runs. Permissions you revoked stay revoked unless you allow them again here."

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose()
      }}
    >
      <DialogContent className="max-h-[90svh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{intro}</DialogDescription>
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-mono text-xs">{request.pluginId}</span>
            <PublisherLine publisher={request.publisher} namespace={request.namespace} />
          </div>
        </DialogHeader>
        {fixtures && (
          <p className="rounded-md border border-dashed px-3 py-2 text-xs text-muted-foreground">
            {FIXTURE_LABEL}. Confirming changes only the in-memory fixture state.
          </p>
        )}
        <p className="flex items-start gap-2 text-xs text-muted-foreground">
          <ShieldAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
          <span>
            DeckPilot is designed to enforce these permissions in its plugin runtime. That runtime
            and its sandbox are experimental and not ready for production use; only install plugins
            you trust.
          </span>
        </p>
        <PermissionList
          label="Requested permissions"
          grants={request.items.map((i) => i.grant)}
          renderControl={(grant) => {
            const state = states.get(grant.key)
            const id = `consent-${grant.key}`
            if (state === "granted") {
              return (
                <Badge variant="secondary">
                  <Check aria-hidden /> Already granted
                </Badge>
              )
            }
            if (state === "revoked" && request.mode !== "enable") {
              return <Badge variant="outline">Revoked; stays revoked</Badge>
            }
            return (
              <div className="flex shrink-0 items-center gap-2">
                <Checkbox
                  id={id}
                  checked={checked.has(grant.key)}
                  disabled={busy}
                  aria-label={`${state === "revoked" ? "Allow again" : "Allow"}: ${grant.title}`}
                  onCheckedChange={(value) => {
                    toggle(grant.key, value === true)
                  }}
                />
                <label htmlFor={id} className="text-xs">
                  {state === "revoked"
                    ? "Allow again"
                    : state === "new" && request.mode === "update"
                      ? "New: allow"
                      : "Allow"}
                </label>
              </div>
            )
          }}
        />
        {request.removed && request.removed.length > 0 && (
          <div className="grid gap-1 text-sm">
            <h4 className="text-xs font-medium text-muted-foreground uppercase">
              No longer requested
            </h4>
            <ul className="grid gap-0.5" aria-label="No longer requested">
              {request.removed.map((title) => (
                <li key={title}>{title}</li>
              ))}
            </ul>
          </div>
        )}
        {error && (
          <Alert variant="destructive">
            <AlertTitle className="line-clamp-none">{error.message}</AlertTitle>
            {error.hint && <AlertDescription>{error.hint}</AlertDescription>}
          </Alert>
        )}
        <DialogFooter className="items-center gap-2 sm:justify-between">
          <p className="text-xs text-muted-foreground" aria-live="polite">
            {required.length === 0
              ? request.mode === "enable"
                ? "No new permissions are needed."
                : "This plugin asks for no permissions."
              : missing === 0
                ? "You consented to every new permission."
                : `${String(missing)} of ${String(required.length)} new permissions still need your consent.`}
          </p>
          <div className="flex flex-wrap gap-2">
            {required.length > 1 && (
              <Button
                variant="outline"
                disabled={busy || missing === 0}
                onClick={() => {
                  setChecked(new Set([...checked, ...required]))
                }}
              >
                Allow all {required.length}
              </Button>
            )}
            <Button variant="ghost" disabled={busy} onClick={onClose}>
              Cancel
            </Button>
            <Button disabled={busy || missing > 0} onClick={submit}>
              {busy ? "Working…" : CONFIRM[request.mode]}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
