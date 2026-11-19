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

import { pluginErrorText } from "./api"
import { formatTime } from "./model"
import type { PluginSettings, PluginUserOrigin, UserOriginStatus } from "./types"
import { consentText, listed } from "./user-origins-text"

const STATUS: Record<
  UserOriginStatus,
  { label: string; variant: "secondary" | "outline" | "destructive" }
> = {
  unset: { label: "Not set", variant: "outline" },
  invalid: { label: "Invalid", variant: "destructive" },
  declared: { label: "Declared", variant: "secondary" },
  slotNotGranted: { label: "Not permitted", variant: "outline" },
  needsConsent: { label: "Needs your consent", variant: "destructive" },
  granted: { label: "Allowed", variant: "secondary" },
}

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
  return (
    <section className="grid gap-3" aria-labelledby="configure-servers">
      <h4 id="configure-servers" className="text-xs font-medium text-muted-foreground uppercase">
        Servers you enter
      </h4>
      <p className="text-xs text-muted-foreground">
        The plugin reaches the server in these settings only after you allow exactly that server.
        Changing a setting ends its consent; DeckPilot then asks again.
      </p>
      <ul className="grid divide-y border-y" aria-label={`Servers of ${plugin}`}>
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
        const text = pluginErrorText(e)
        setError(`${text.title}: ${text.message}`)
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
        <Badge variant={status.variant}>{status.label}</Badge>
        {origin.status === "granted" && origin.privateNetworkGranted && (
          <Badge variant="outline">Private network</Badge>
        )}
        {origin.status === "granted" && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="ml-auto h-7"
            disabled={busy}
            aria-label={`Revoke ${origin.label}`}
            onClick={() => {
              run(() => revoke(origin.setting), `${origin.label}: consent revoked`)
            }}
          >
            Revoke
          </Button>
        )}
      </div>
      {origin.origin && <p className="font-mono text-xs break-all select-text">{origin.origin}</p>}
      {origin.status === "unset" && (
        <p className="text-xs text-muted-foreground">
          No server is set, so the plugin reaches none through this setting.
        </p>
      )}
      {origin.status === "invalid" && (
        <p className="text-xs text-destructive">
          The setting {origin.problem ?? "is not a server this plugin can use"}.
        </p>
      )}
      {origin.status === "declared" && (
        <p className="text-xs text-muted-foreground">
          A host the plugin declares, allowed when you installed it.
        </p>
      )}
      {origin.status === "slotNotGranted" && (
        <p className="text-xs text-muted-foreground">
          The plugin's permission to connect to a server you enter is not granted. Grant “
          {origin.permission}” first; nothing goes to this server until then.
        </p>
      )}
      {origin.status === "granted" && (
        <p className="text-xs text-muted-foreground">
          Allowed{origin.grantedAt !== undefined ? ` ${formatTime(origin.grantedAt)}` : ""}
          {origin.privateNetworkGranted ? ", on your private network" : ""}.
          {origin.credentials?.length
            ? ` ${listed(origin.credentials.map((c) => `“${c}”`))} ${origin.credentials.length === 1 ? "is" : "are"} sent there.`
            : ""}
        </p>
      )}
      {origin.status === "needsConsent" && origin.origin && (
        <div
          className="grid gap-2 rounded-md bg-muted/50 p-2"
          role="group"
          aria-label={`Allow ${origin.origin}?`}
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
                  Also allow it on my private network
                </label>
                <p id={`${id}-private-help`} className="text-xs text-muted-foreground">
                  {needsPrivate
                    ? "Required: this is a local name or address. "
                    : "Only if the server resolves to an address on your network. "}
                  This computer and link-local addresses are never reachable.
                </p>
              </div>
            </div>
          )}
          {needsPrivate && !origin.privateNetworkAllowed && (
            <p className="text-xs text-destructive">
              This server is on a private network, which this plugin's setting does not allow.
            </p>
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
                  `${origin.label}: ${server} allowed`,
                )
              }}
            >
              {busy ? "Allowing…" : "Allow"}
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
