// Plugin permissions in plain language: normalization into grants
// (architecture §5.4), risk tiers (§5.2) and the comparison an update makes
// with the stored consent (§13.2).
//
// The wording describes what DeckPilot is designed to enforce. Nothing
// enforces it yet: the plugin runtime and its sandbox are not implemented.

import type { PermissionKind, PluginPermission, StoredGrant } from "./types"

export type RiskTier = "high" | "medium" | "low"

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

/** One grant the user consents to, revokes and is asked about again on updates. */
export interface Grant {
  key: string
  kind: PermissionKind
  tier: RiskTier
  title: string
  detail: string
  justification: string
  permission: PluginPermission
}

/** `https://host[:port]`, or a description of a wildcard pattern. */
export function describeHost(pattern: string): string {
  if (pattern.startsWith("*.")) {
    return `any subdomain of ${pattern.slice(2)} (https://*.${pattern.slice(2)})`
  }
  return `https://${pattern}`
}

const ALL_METHODS = "any HTTP method the plugin chooses (GET, HEAD, POST, PUT, PATCH, DELETE)"

/** The grants of a manifest's `permissions`, one per host, ref or read entry. */
export function grantsOf(permissions: PluginPermission[]): Grant[] {
  return permissions.flatMap((permission): Grant[] => {
    const { justification } = permission
    switch (permission.kind) {
      case "network": {
        const methods = permission.methods ? permission.methods.join(", ") : ALL_METHODS
        const hosts = permission.hosts.map((host): Grant => ({
          key: `network:${host}`,
          kind: "network",
          tier: "medium",
          title: `Send data to and receive data from ${describeHost(host)}`,
          detail: `HTTPS only. Methods: ${methods}. Whatever the plugin can read can be sent to this host.`,
          justification,
          permission,
        }))
        if (!permission.privateNetwork) return hosts
        return [
          ...hosts,
          {
            key: "network:private",
            kind: "network",
            tier: "high",
            title: "Connect to devices on your local network that you add",
            detail:
              "Only hosts you enter yourself in the plugin's settings; none until you add one. Plain HTTP is allowed for them.",
            justification,
            permission,
          },
        ]
      }
      case "credentials":
        return permission.refs.map((ref) => ({
          key: `credentials:${ref}`,
          kind: "credentials",
          tier: "high",
          title: `Use the secret “${ref}” that you store for it`,
          detail:
            "DeckPilot adds the secret to the plugin's requests, only to the hosts declared for this secret. The plugin gets a reference, not the value. You enter the secret after installing; it is never shown here.",
          justification,
          permission,
        }))
      case "notifications":
        return [
          {
            key: "notifications",
            kind: "notifications",
            tier: "low",
            title: "Show notifications",
            detail: "Prefixed with the plugin's name; at most 5 per minute.",
            justification,
            permission,
          },
        ]
      case "profile":
        return [
          {
            key: "profile",
            kind: "profile",
            tier: "medium",
            title: `Read the active ${permission.read.join(" and ")}`,
            detail: "Id, key and name only; not their contents.",
            justification,
            permission,
          },
        ]
      case "variables":
        return permission.read.map((id) => ({
          key: `variables:${id}`,
          kind: "variables",
          tier: "medium",
          title: id.endsWith(".*")
            ? `Read the variables under ${id.slice(0, -2)}`
            : `Read the variable ${id}`,
          detail: "Sensitive variables of other owners and secret values are never readable.",
          justification,
          permission,
        }))
      case "storage":
        return [
          {
            key: "storage",
            kind: "storage",
            tier: "low",
            title: `Store up to ${permission.quotaKb} KB of its own data`,
            detail: "Raises the plugin's own storage above the default of 256 KB.",
            justification,
            permission,
          },
        ]
      case "openUrl":
        return permission.hosts.map((host) => ({
          key: `openUrl:${host}`,
          kind: "openUrl",
          tier: "medium",
          title: `Open links to ${describeHost(host)} in your browser`,
          detail: "Only right after a key press or an action in DeckPilot.",
          justification,
          permission,
        }))
      case "background":
        return [
          {
            key: "background",
            kind: "background",
            tier: "medium",
            title: "Run in the background",
            detail: `Timers at most every ${permission.minIntervalSeconds} seconds; the plugin's process keeps running.`,
            justification,
            permission,
          },
        ]
      case "device": {
        const parts = [
          permission.state && "change what its own keys show",
          permission.icons && "use its own icons on keys",
        ].filter(Boolean)
        return [
          {
            key: "device",
            kind: "device",
            tier: "low",
            title: parts.length ? `Stream Deck: ${parts.join(" and ")}` : "Stream Deck keys",
            detail: "Only keys bound to the plugin's own actions.",
            justification,
            permission,
          },
        ]
      }
    }
  })
}

export function grantsByTier<T extends { tier: RiskTier }>(grants: T[]) {
  return TIERS.map((tier) => ({
    ...tier,
    grants: grants.filter((g) => g.tier === tier.tier),
  })).filter((group) => group.grants.length > 0)
}

/** Whether `next` asks for more than `previous` under the same key (§13.2). */
function broadens(previous: PluginPermission, next: PluginPermission): boolean {
  if (previous.kind === "storage" && next.kind === "storage") {
    return next.quotaKb > previous.quotaKb
  }
  if (previous.kind === "background" && next.kind === "background") {
    return next.minIntervalSeconds < previous.minIntervalSeconds
  }
  if (previous.kind === "device" && next.kind === "device") {
    return (Boolean(next.state) && !previous.state) || (Boolean(next.icons) && !previous.icons)
  }
  if (previous.kind === "profile" && next.kind === "profile") {
    return next.read.some((r) => !previous.read.includes(r))
  }
  if (previous.kind === "network" && next.kind === "network") {
    const before = previous.methods
    if (!before) return false
    return !next.methods || next.methods.some((m) => !before.includes(m))
  }
  return false
}

export interface GrantChange {
  /** Grants the candidate asks for that the user has not consented to. */
  added: Grant[]
  /** Grants the candidate keeps and the user already holds (granted or revoked). */
  kept: { grant: Grant; stored: StoredGrant }[]
  /** Stored grants the candidate no longer asks for; they are deleted. */
  removed: StoredGrant[]
}

/**
 * Compares a candidate version's permissions with the stored consent.
 * Anything in `added` is an escalation and needs the user's approval.
 */
export function compareGrants(stored: StoredGrant[], candidate: PluginPermission[]): GrantChange {
  const next = grantsOf(candidate)
  const byKey = new Map(stored.map((g) => [g.key, g]))
  const added: Grant[] = []
  const kept: GrantChange["kept"] = []
  for (const grant of next) {
    const previous = byKey.get(grant.key)
    if (!previous || broadens(previous.permission, grant.permission)) added.push(grant)
    else kept.push({ grant, stored: previous })
  }
  const keys = new Set(next.map((g) => g.key))
  return { added, kept, removed: stored.filter((g) => !keys.has(g.key)) }
}
