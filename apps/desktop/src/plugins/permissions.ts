// Plugin permissions in plain language.
//
// Grant keys are the agent's (`deckpilot-plugin-store` `GrantSet`):
// `network:<host>`, `network:private-network`, `credentials:<ref>`,
// `notifications`, `profile`, `variables:<entry>`, `storage`,
// `openUrl:<host>`, `background`, `device:state`, `device:icons`. A
// consent names exactly these keys, so `grantsOf` must normalize a
// manifest the way the agent does. Installed plugins come with the
// agent's own normalization (`GrantInfo`) and are only described here.

import type { GrantInfo, ManifestPermission, PermissionTier } from "./types"

export type RiskTier = PermissionTier

export const TIERS: { tier: RiskTier; label: string; description: string }[] = [
  {
    tier: "high",
    label: "High risk",
    description: "Access to secrets or to devices on your local network.",
  },
  {
    tier: "medium",
    label: "Medium risk",
    description: "Data leaves this computer or the plugin reads your configuration.",
  },
  { tier: "low", label: "Low risk", description: "Limited to the plugin's own resources." },
]

/** The key of the grant `privateNetwork: true` adds. */
export const PRIVATE_NETWORK_KEY = "network:private-network"

/** One grant the user consents to and can revoke. */
export interface Grant {
  key: string
  tier: RiskTier
  title: string
  detail: string
  justification?: string
}

/** `https://host[:port]`, or a description of a wildcard pattern. */
export function describeHost(pattern: string): string {
  if (pattern.startsWith("*.")) {
    return `any subdomain of ${pattern.slice(2)} (https://*.${pattern.slice(2)})`
  }
  return `https://${pattern}`
}

const ALL_METHODS = "any HTTP method the plugin chooses (GET, HEAD, POST, PUT, PATCH, DELETE)"

interface Facts {
  methods?: string[]
  read?: string[]
  quotaKb?: number
  minIntervalSeconds?: number
}

/** Title, detail and tier of one grant key. */
function describe(key: string, facts: Facts): Omit<Grant, "key" | "justification"> {
  const colon = key.indexOf(":")
  const kind = colon < 0 ? key : key.slice(0, colon)
  const target = colon < 0 ? "" : key.slice(colon + 1)
  switch (kind) {
    case "network":
      if (key === PRIVATE_NETWORK_KEY) {
        return {
          tier: "high",
          title: "Connect to devices on your local network that you add",
          detail:
            "Only hosts you enter yourself in the plugin's settings; none until you add one. Plain HTTP is allowed for them.",
        }
      }
      return {
        tier: "medium",
        title: `Send data to and receive data from ${describeHost(target)}`,
        detail: `HTTPS only. Methods: ${facts.methods?.length ? facts.methods.join(", ") : ALL_METHODS}. Whatever the plugin can read can be sent to this host.`,
      }
    case "credentials":
      return {
        tier: "high",
        title: `Use the secret “${target}” that you store for it`,
        detail:
          "DeckPilot adds the secret to the plugin's requests, only to the hosts declared for this secret. The plugin gets a reference, not the value.",
      }
    case "notifications":
      return {
        tier: "low",
        title: "Show notifications",
        detail: "Prefixed with the plugin's name.",
      }
    case "profile":
      return {
        tier: "medium",
        title: `Read the active ${facts.read?.length ? facts.read.join(" and ") : "profile"}`,
        detail: "Id, key and name only; not their contents.",
      }
    case "variables":
      return {
        tier: "medium",
        title: target.endsWith(".*")
          ? `Read the variables under ${target.slice(0, -2)}`
          : `Read the variable ${target}`,
        detail: "Sensitive variables of other owners and secret values are never readable.",
      }
    case "storage":
      return {
        tier: "low",
        title:
          facts.quotaKb === undefined
            ? "Store its own data"
            : `Store up to ${String(facts.quotaKb)} KB of its own data`,
        detail: "Raises the plugin's own storage above the default.",
      }
    case "openUrl":
      return {
        tier: "medium",
        title: `Open links to ${describeHost(target)} in your browser`,
        detail: "Only right after a key press or an action in DeckPilot.",
      }
    case "background":
      return {
        tier: "medium",
        title: "Run in the background",
        detail:
          facts.minIntervalSeconds === undefined
            ? "The plugin's process keeps running."
            : `Timers at most every ${String(facts.minIntervalSeconds)} seconds; the plugin's process keeps running.`,
      }
    case "device":
      return {
        tier: "low",
        title:
          target === "icons"
            ? "Stream Deck: use its own icons on keys"
            : "Stream Deck: change what its own keys show",
        detail: "Only keys bound to the plugin's own actions.",
      }
    default:
      return { tier: "medium", title: key, detail: "" }
  }
}

/**
 * The grants of a manifest's `permissions` (a registry version's), keyed as
 * the agent keys them. Duplicate keys are merged.
 */
export function grantsOf(permissions: ManifestPermission[]): Grant[] {
  const grants = new Map<string, Grant>()
  const add = (key: string, justification: string, facts: Facts = {}) => {
    if (!grants.has(key)) grants.set(key, { key, justification, ...describe(key, facts) })
  }
  for (const permission of permissions) {
    const { justification } = permission
    switch (permission.kind) {
      case "network":
        for (const host of permission.hosts) {
          add(
            `network:${host}`,
            justification,
            permission.methods ? { methods: permission.methods } : {},
          )
        }
        if (permission.privateNetwork) add(PRIVATE_NETWORK_KEY, justification)
        break
      case "credentials":
        for (const ref of permission.refs) add(`credentials:${ref}`, justification)
        break
      case "notifications":
        add("notifications", justification)
        break
      case "profile":
        add("profile", justification, { read: permission.read })
        break
      case "variables":
        for (const entry of permission.read) add(`variables:${entry}`, justification)
        break
      case "storage":
        add("storage", justification, { quotaKb: permission.quotaKb })
        break
      case "openUrl":
        for (const host of permission.hosts) add(`openUrl:${host}`, justification)
        break
      case "background":
        add("background", justification, { minIntervalSeconds: permission.minIntervalSeconds })
        break
      case "device":
        if (permission.state) add("device:state", justification)
        if (permission.icons) add("device:icons", justification)
        break
    }
  }
  return [...grants.values()]
}

function scopeFacts(scope: unknown): Facts {
  if (typeof scope !== "object" || scope === null) return {}
  const s = scope as Record<string, unknown>
  const strings = (v: unknown) =>
    Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : undefined
  const facts: Facts = {}
  const methods = strings(s.methods)
  if (methods) facts.methods = methods
  const read = strings(s.read)
  if (read) facts.read = read
  if (typeof s.quotaKb === "number") facts.quotaKb = s.quotaKb
  if (typeof s.minIntervalSeconds === "number") facts.minIntervalSeconds = s.minIntervalSeconds
  return facts
}

/** A permission as the agent reports it, in plain language; its tier is the agent's. */
export function grantOfInfo(info: GrantInfo): Grant {
  const grant: Grant = { key: info.key, ...describe(info.key, scopeFacts(info.scope)) }
  grant.tier = info.tier
  if (info.justification) grant.justification = info.justification
  return grant
}

export function grantsByTier<T extends { tier: RiskTier }>(grants: T[]) {
  return TIERS.map((tier) => ({
    ...tier,
    grants: grants.filter((g) => g.tier === tier.tier),
  })).filter((group) => group.grants.length > 0)
}
