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

import { pluginErrorText, type PluginErrorText } from "./api"
import { PermissionList, PublisherLine } from "./components"
import type { Grant } from "./permissions"
import type { PublisherInfo } from "./types"

/**
 * - `new`: needs the user's consent now.
 * - `broadened`: an update asks for more under a key already granted.
 * - `granted`: consent is stored; shown for context.
 */
export type ReviewState = "new" | "broadened" | "granted"

export interface ReviewRequest {
  mode: "install" | "update" | "enable"
  pluginId: string
  name: string
  publisher: string
  /** The registry's record of the publisher, for registry installs. */
  publisherInfo?: PublisherInfo | undefined
  version: string
  items: { grant: Grant; state: ReviewState }[]
  /** Grant keys an update drops; shown so the change is complete. */
  removed?: string[]
  /**
   * Called with exactly the keys the user checked. Resolves when done; a
   * rejection is shown in the dialog.
   */
  confirm: (consent: string[]) => Promise<void>
  /** Updates only: drops the staged version (`declinePluginUpdate`). */
  decline?: () => Promise<void>
}

const CONFIRM = { install: "Install", update: "Update", enable: "Enable" } as const

/**
 * Asks for consent before an install, an update that asks for more, or
 * enabling a plugin. Every permission that needs consent has its own
 * check; "Allow all" is a separate, explicit action. The consent sent is
 * exactly the set of checked keys. The dialog keeps focus until closed.
 */
export function PermissionReviewDialog({
  request,
  onClose,
}: {
  request: ReviewRequest
  onClose: () => void
}) {
  const [checked, setChecked] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<PluginErrorText | null>(null)
  const required = request.items.filter((i) => i.state !== "granted").map((i) => i.grant.key)
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

  const finish = (work: Promise<void>) => {
    setBusy(true)
    setError(null)
    work.then(onClose, (e: unknown) => {
      setError(pluginErrorText(e))
      setBusy(false)
    })
  }

  const submit = () => {
    finish(request.confirm(required.filter((key) => checked.has(key))))
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
        ? "This version asks for more than you granted. Check each new or broadened permission to consent, decline the update, or cancel to decide later."
        : "A plugin runs only with every permission it declares. Check each one that is not granted yet."

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
            <PublisherLine publisher={request.publisher} info={request.publisherInfo} />
          </div>
        </DialogHeader>
        <p className="flex items-start gap-2 text-xs text-muted-foreground">
          <ShieldAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
          <span>
            The agent enforces these permissions on every call the plugin makes. The plugin runtime
            is experimental; only install plugins you trust.
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
            return (
              <div className="flex shrink-0 items-center gap-2">
                <Checkbox
                  id={id}
                  checked={checked.has(grant.key)}
                  disabled={busy}
                  aria-label={`Allow: ${grant.title}`}
                  onCheckedChange={(value) => {
                    toggle(grant.key, value === true)
                  }}
                />
                <label htmlFor={id} className="text-xs">
                  {state === "broadened"
                    ? "Broadened: allow"
                    : request.mode === "update"
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
            <ul className="grid gap-0.5 font-mono text-xs" aria-label="No longer requested">
              {request.removed.map((key) => (
                <li key={key}>{key}</li>
              ))}
            </ul>
          </div>
        )}
        {error && (
          <Alert variant="destructive">
            <AlertTitle className="line-clamp-none">{error.title}</AlertTitle>
            <AlertDescription className="grid gap-1">
              <p>{error.message}</p>
              {error.hint && <p>{error.hint}</p>}
            </AlertDescription>
          </Alert>
        )}
        <DialogFooter className="items-center gap-2 sm:justify-between">
          <p className="text-xs text-muted-foreground" aria-live="polite">
            {required.length === 0
              ? request.mode === "enable"
                ? "Every permission is granted."
                : "This plugin asks for no permissions."
              : missing === 0
                ? "You consented to every permission."
                : `${String(missing)} of ${String(required.length)} permissions still need your consent.`}
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
            {request.decline && (
              <Button
                variant="outline"
                disabled={busy}
                onClick={() => {
                  if (request.decline) finish(request.decline())
                }}
              >
                Decline update
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
