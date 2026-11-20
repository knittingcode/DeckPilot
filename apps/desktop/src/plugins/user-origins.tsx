// Servers the user enters (G2): the origin settings of a plugin, and the
// consent to exactly the server each one holds, against the agent's
// `grantPluginUserOrigin` / `revokePluginUserOrigin`.
//
// The consent names the server as the agent normalized it, never what the
// form holds. A private network is a separate, explicit choice: never
// pre-selected, required for a local name or address, and its own line in
// the agent's record. Changing the setting ends the consent (the agent
// revokes it), so the row asks again.

import { useId, useState } from "react"
import { toast } from "sonner"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { i18n, type TranslationKey, useTranslation } from "@/i18n"

import { pluginErrorText } from "./api"
import { errorSentence } from "./dialog-error"
import { formatTime } from "./model"
import type { PluginSettings, PluginUserOrigin, UserOriginStatus } from "./types"
import { consentText, quotedList } from "./user-origins-text"

const STATUS = {
  unset: { label: "plugins:settings.notSet", variant: "outline" },
  invalid: { label: "plugins:origins.status.invalid", variant: "destructive" },
  declared: { label: "plugins:origins.status.declared", variant: "secondary" },
  slotNotGranted: { label: "plugins:origins.status.slotNotGranted", variant: "outline" },
  needsConsent: { label: "plugins:origins.status.needsConsent", variant: "destructive" },
  granted: { label: "plugins:origins.status.granted", variant: "secondary" },
} as const satisfies Record<
  UserOriginStatus,
  { label: TranslationKey; variant: "secondary" | "outline" | "destructive" }
>

export function UserOriginsSection({
  plugin,
  origins,
  grant,
  revoke,
  onSettings,
}: {
  /** The plugin's name. */
  plugin: string
  origins: PluginUserOrigin[]
  grant: (setting: string, origin: string, privateNetwork: boolean) => Promise<PluginSettings>
  revoke: (setting: string) => Promise<PluginSettings>
  /** The agent's answer after a grant or a revocation. */
  onSettings: (settings: PluginSettings) => void
}) {
  const { t } = useTranslation()
  return (
    <section className="grid gap-3" aria-labelledby="configure-servers">
      <h4 id="configure-servers" className="text-xs font-medium text-muted-foreground uppercase">
        {t("plugins:origins.title")}
      </h4>
      <p className="text-xs text-muted-foreground">{t("plugins:origins.intro")}</p>
      <ul
        className="grid divide-y border-y"
        aria-label={t("plugins:origins.list", { name: plugin })}
      >
        {origins.map((origin) => (
          <OriginRow
            key={origin.setting}
            plugin={plugin}
            origin={origin}
            grant={grant}
            revoke={revoke}
            onSettings={onSettings}
          />
        ))}
      </ul>
    </section>
  )
}

function OriginRow({
  plugin,
  origin,
  grant,
  revoke,
  onSettings,
}: {
  plugin: string
  origin: PluginUserOrigin
  grant: (setting: string, origin: string, privateNetwork: boolean) => Promise<PluginSettings>
  revoke: (setting: string) => Promise<PluginSettings>
  onSettings: (settings: PluginSettings) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  // The private network is never assumed: unticked for every new server.
  const [privateNetwork, setPrivateNetwork] = useState(false)
  const [shownFor, setShownFor] = useState(origin.origin)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  if (shownFor !== origin.origin) {
    setShownFor(origin.origin)
    setPrivateNetwork(false)
    setError(null)
  }
  const status = STATUS[origin.status]
  const needsPrivate = origin.privateHost === true

  const run = (action: () => Promise<PluginSettings>, done: string) => {
    setBusy(true)
    setError(null)
    action()
      .then((next) => {
        onSettings(next)
        toast.success(done)
      })
      .catch((e: unknown) => {
        setError(errorSentence(pluginErrorText(e), false))
      })
      .finally(() => {
        setBusy(false)
      })
  }

  return (
    <li className="grid gap-2 py-2.5" aria-labelledby={`${id}-label`}>
      <div className="flex flex-wrap items-center gap-2">
        <span id={`${id}-label`} className="text-sm font-medium">
          {origin.label}
        </span>
        <Badge variant={status.variant}>{t(status.label)}</Badge>
        {origin.status === "granted" && origin.privateNetworkGranted && (
          <Badge variant="outline">{t("plugins:origins.privateNetwork")}</Badge>
        )}
        {origin.status === "granted" && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="ml-auto h-7"
            disabled={busy}
            aria-label={t("plugins:origins.revokeNamed", { label: origin.label })}
            onClick={() => {
              run(
                () => revoke(origin.setting),
                i18n.t("plugins:origins.revoked", { label: origin.label }),
              )
            }}
          >
            {t("marketplace:installed.revoke")}
          </Button>
        )}
      </div>
      {origin.origin && <p className="font-mono text-xs break-all select-text">{origin.origin}</p>}
      {origin.status === "unset" && (
        <p className="text-xs text-muted-foreground">{t("plugins:origins.unset")}</p>
      )}
      {origin.status === "invalid" && (
        <p className="text-xs text-destructive">
          {origin.problem
            ? t("plugins:origins.invalidBecause", { problem: origin.problem })
            : t("plugins:origins.invalid")}
        </p>
      )}
      {origin.status === "declared" && (
        <p className="text-xs text-muted-foreground">{t("plugins:origins.declared")}</p>
      )}
      {origin.status === "slotNotGranted" && (
        <p className="text-xs text-muted-foreground">
          {t("plugins:origins.slotNotGranted", { permission: origin.permission })}
        </p>
      )}
      {origin.status === "granted" && (
        <p className="text-xs text-muted-foreground">{grantedText(origin)}</p>
      )}
      {origin.status === "needsConsent" && origin.origin && (
        <div
          className="grid gap-2 rounded-md bg-muted/50 p-2"
          role="group"
          aria-label={t("plugins:origins.allowAsk", { server: origin.origin })}
        >
          <p className="text-sm">{consentText(plugin, origin)}</p>
          {origin.privateNetworkAllowed && (
            <div className="flex items-start gap-2">
              <Checkbox
                id={`${id}-private`}
                checked={privateNetwork}
                disabled={busy}
                aria-describedby={`${id}-private-help`}
                onCheckedChange={(checked) => {
                  setPrivateNetwork(checked === true)
                }}
              />
              <div className="grid gap-1">
                <label htmlFor={`${id}-private`} className="text-sm leading-none font-medium">
                  {t("plugins:origins.alsoPrivate")}
                </label>
                <p id={`${id}-private-help`} className="text-xs text-muted-foreground">
                  {needsPrivate
                    ? t("plugins:origins.privateRequired")
                    : t("plugins:origins.privateOptional")}{" "}
                  {t("plugins:origins.neverLocal")}
                </p>
              </div>
            </div>
          )}
          {needsPrivate && !origin.privateNetworkAllowed && (
            <p className="text-xs text-destructive">{t("plugins:origins.privateNotAllowed")}</p>
          )}
          <div>
            <Button
              type="button"
              size="sm"
              disabled={busy || (needsPrivate && !privateNetwork)}
              onClick={() => {
                const server = origin.origin
                if (!server) return
                run(
                  () => grant(origin.setting, server, privateNetwork),
                  i18n.t("plugins:origins.allowed", { label: origin.label, server }),
                )
              }}
            >
              {busy ? t("plugins:origins.allowing") : t("permissions:review.allow")}
            </Button>
          </div>
        </div>
      )}
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </li>
  )
}

/** What a granted server's consent covers, in words. */
function grantedText(origin: PluginUserOrigin): string {
  const at = origin.grantedAt !== undefined ? formatTime(origin.grantedAt) : null
  const allowed = origin.privateNetworkGranted
    ? at
      ? i18n.t("plugins:origins.grantedPrivateAt", { time: at })
      : i18n.t("plugins:origins.grantedPrivate")
    : at
      ? i18n.t("plugins:origins.grantedAt", { time: at })
      : i18n.t("plugins:origins.grantedPlain")
  const credentials = origin.credentials ?? []
  return credentials.length > 0
    ? `${allowed} ${i18n.t("plugins:origins.secretsSent", {
        count: credentials.length,
        secrets: quotedList(credentials),
      })}`
    : allowed
}
