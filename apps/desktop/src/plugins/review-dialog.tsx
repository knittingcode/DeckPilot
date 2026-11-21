import { Check, Loader2, ShieldAlert, ShieldCheck } from "lucide-react"
import { useState } from "react"

import { ModalShell } from "@/components/dialogs"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { useTranslation } from "@/i18n"

import { PermissionList, PublisherLine } from "./components"
import { pluginErrorSentence } from "./dialog-error"
import { type Grant, optionalOnByDefault } from "./permissions"
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

const CONFIRM = {
  install: "permissions:review.confirm.install",
  update: "permissions:review.confirm.update",
  enable: "permissions:review.confirm.enable",
} as const

/**
 * Asks for consent before an install, an update that asks for more, or
 * enabling a plugin. Every permission that needs consent has its own
 * check; "Allow all" is a separate, explicit action. An optional
 * permission (plugin API 1.9: only some actions need it) has its own check
 * too but never blocks the confirm; it starts checked unless it is high
 * tier, which stays off until the user allows it. The consent sent is
 * exactly the set of checked keys. The dialog keeps focus until closed.
 */
export function PermissionReviewDialog({
  request,
  onClose,
}: {
  request: ReviewRequest
  onClose: () => void
}) {
  const { t } = useTranslation()
  const pending = request.items.filter((i) => i.state !== "granted")
  const [checked, setChecked] = useState<Set<string>>(
    () => new Set(pending.filter((i) => optionalOnByDefault(i.grant)).map((i) => i.grant.key)),
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const required = pending.filter((i) => !i.grant.optionalFor).map((i) => i.grant.key)
  const optional = pending.filter((i) => i.grant.optionalFor).map((i) => i.grant.key)
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
      setError(pluginErrorSentence(e))
      setBusy(false)
    })
  }

  const submit = () => {
    finish(request.confirm([...required, ...optional].filter((key) => checked.has(key))))
  }

  const names = { name: request.name, version: request.version }
  const title =
    request.mode === "install"
      ? t("permissions:review.title.install", names)
      : request.mode === "update"
        ? t("permissions:review.title.update", names)
        : t("permissions:review.title.enable", names)
  const intro =
    request.mode === "install"
      ? t("permissions:review.intro.install")
      : request.mode === "update"
        ? t("permissions:review.intro.update")
        : t("permissions:review.intro.enable")

  const status = (
    <span aria-live="polite">
      {required.length === 0
        ? request.mode === "enable"
          ? t("permissions:review.status.allGranted")
          : t("permissions:list.none")
        : missing === 0
          ? t("permissions:review.status.consented")
          : t("permissions:review.status.missing", { missing, total: required.length })}
    </span>
  )
  const primary = {
    label: t(CONFIRM[request.mode]),
    loadingLabel: t("common:status.working"),
    loading: busy,
    disabled: missing > 0,
  }

  return (
    <ModalShell
      presentation="dialog"
      size="medium"
      icon={ShieldCheck}
      title={title}
      description={intro}
      error={error}
      status={status}
      onClose={() => {
        if (!busy) onClose()
      }}
      onSubmit={submit}
      primary={primary}
      secondary={{ label: t("common:actions.cancel"), onClick: onClose, disabled: busy }}
      // An update that asks for more can also be declined: a third action.
      footer={
        request.decline ? (
          <>
            <Button
              type="button"
              variant="outline"
              disabled={busy}
              onClick={() => {
                if (request.decline) finish(request.decline())
              }}
            >
              {t("permissions:review.decline")}
            </Button>
            <Button type="button" variant="outline" disabled={busy} onClick={onClose}>
              {t("common:actions.cancel")}
            </Button>
            <Button type="submit" disabled={busy || missing > 0} aria-busy={busy || undefined}>
              {busy && <Loader2 className="animate-spin" aria-hidden />}
              {busy ? primary.loadingLabel : primary.label}
            </Button>
          </>
        ) : undefined
      }
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-xs">{request.pluginId}</span>
        <PublisherLine publisher={request.publisher} info={request.publisherInfo} />
      </div>
      <p className="flex items-start gap-2 text-xs text-muted-foreground">
        <ShieldAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
        <span>{t("permissions:review.enforced")}</span>
      </p>
      {required.length > 1 && (
        <div className="flex justify-end">
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={busy || missing === 0}
            onClick={() => {
              setChecked(new Set([...checked, ...required]))
            }}
          >
            {t("permissions:review.allowAll", { number: required.length })}
          </Button>
        </div>
      )}
      <PermissionList
        label={t("permissions:review.requested")}
        grants={request.items.map((i) => i.grant)}
        renderControl={(grant) => {
          const state = states.get(grant.key)
          const id = `consent-${grant.key}`
          if (state === "granted") {
            return (
              <Badge variant="secondary">
                <Check aria-hidden /> {t("permissions:review.alreadyGranted")}
              </Badge>
            )
          }
          return (
            <div className="flex shrink-0 items-center gap-2">
              <Checkbox
                id={id}
                checked={checked.has(grant.key)}
                disabled={busy}
                aria-label={t("permissions:review.allowNamed", { title: grant.title })}
                onCheckedChange={(value) => {
                  toggle(grant.key, value === true)
                }}
              />
              <label htmlFor={id} className="text-xs">
                {state === "broadened"
                  ? t("permissions:review.allowBroadened")
                  : grant.optionalFor
                    ? t("permissions:review.allowOptional")
                    : request.mode === "update"
                      ? t("permissions:review.allowNew")
                      : t("permissions:review.allow")}
              </label>
            </div>
          )
        }}
      />
      {request.removed && request.removed.length > 0 && (
        <div className="grid gap-1 text-sm">
          <h4 className="text-xs font-medium text-muted-foreground uppercase">
            {t("permissions:review.removed")}
          </h4>
          <ul
            className="grid gap-0.5 font-mono text-xs"
            aria-label={t("permissions:review.removed")}
          >
            {request.removed.map((key) => (
              <li key={key}>{key}</li>
            ))}
          </ul>
        </div>
      )}
    </ModalShell>
  )
}
