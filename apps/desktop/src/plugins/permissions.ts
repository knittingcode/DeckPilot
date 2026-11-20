// Plugin permissions in plain language.
//
// Grant keys are the agent's (`deckpilot-plugin-store` `GrantSet`):
// `network:<host>`, `network:private-network`,
// `network:user-origin:<setting>`, `credentials:<ref>`,
// `notifications`, `profile`, `variables:<entry>`, `storage`,
// `openUrl:<host>`, `background`, `device:state`, `device:icons`. A
// consent names exactly these keys, so `grantsOf` must normalize a
// manifest the way the agent does. Installed plugins come with the
// agent's own normalization (`GrantInfo`) and are only described here.

import { type TranslationKey, i18n } from "@/i18n"

import type { GrantInfo, ManifestPermission, PermissionTier } from "./types"

export type RiskTier = PermissionTier

const TIER_KEYS = [
  {
    tier: "high",
    label: "permissions:tier.high.label",
    description: "permissions:tier.high.description",
  },
  {
    tier: "medium",
    label: "permissions:tier.medium.label",
    description: "permissions:tier.medium.description",
  },
  {
    tier: "low",
    label: "permissions:tier.low.label",
    description: "permissions:tier.low.description",
  },
] as const satisfies { tier: RiskTier; label: TranslationKey; description: TranslationKey }[]

/** The risk tiers, highest first, with their names in the window's language. */
export function tiers(): { tier: RiskTier; label: string; description: string }[] {
  return TIER_KEYS.map(({ tier, label, description }) => ({
    tier,
    label: i18n.t(label),
    description: i18n.t(description),
  }))
}

/** The name of a risk tier ("High risk"). */
export function tierLabel(tier: RiskTier): string {
  const entry = TIER_KEYS.find((t) => t.tier === tier)
  return entry ? i18n.t(entry.label) : tier
}

/** The key of the grant `privateNetwork: true` adds. */
export const PRIVATE_NETWORK_KEY = "network:private-network"

/** The prefix of an origin setting's grant key (G2). */
export const USER_ORIGIN_PREFIX = "network:user-origin:"
/** The grant to open pages of the server entered in an origin setting. */
export const OPEN_USER_ORIGIN_PREFIX = "openUrl:user-origin:"

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
    return i18n.t("permissions:host.wildcard", { domain: pattern.slice(2) })
  }
  return `https://${pattern}`
}

/** The methods a network grant allows, in words. */
function methodsText(methods: string[] | undefined): string {
  return methods?.length ? methods.join(", ") : i18n.t("permissions:grant.allMethods")
}

interface Facts {
  methods?: string[]
  read?: string[]
  quotaKb?: number
  minIntervalSeconds?: number
  /** Notifications routed by the plugin's settings (`routing`). */
  routed?: boolean
  pathPrefix?: boolean
  privateNetwork?: boolean
}

/** "Read the active profile", "… project", "… profile and project". */
function profileTitle(read: string[] | undefined): string {
  const profile = read?.includes("profile") ?? false
  const project = read?.includes("project") ?? false
  if (profile && project) return i18n.t("permissions:grant.profile.titleBoth")
  if (project) return i18n.t("permissions:grant.profile.titleProject")
  return i18n.t("permissions:grant.profile.titleProfile")
}

/** Title, detail and tier of one grant key, in the window's language. */
function describe(key: string, facts: Facts): Omit<Grant, "key" | "justification"> {
  const colon = key.indexOf(":")
  const kind = colon < 0 ? key : key.slice(0, colon)
  const target = colon < 0 ? "" : key.slice(colon + 1)
  switch (kind) {
    case "network":
      if (key.startsWith(USER_ORIGIN_PREFIX)) {
        const setting = key.slice(USER_ORIGIN_PREFIX.length)
        return {
          tier: facts.privateNetwork ? "high" : "medium",
          title: i18n.t("permissions:grant.userOrigin.title", { setting }),
          detail: [
            i18n.t("permissions:grant.userOrigin.none"),
            i18n.t("permissions:grant.httpsMethods", { methods: methodsText(facts.methods) }),
            facts.pathPrefix ? i18n.t("permissions:grant.userOrigin.pathPrefix") : null,
            facts.privateNetwork ? i18n.t("permissions:grant.userOrigin.privateNetwork") : null,
          ]
            .filter(Boolean)
            .join(" "),
        }
      }
      if (key === PRIVATE_NETWORK_KEY) {
        return {
          tier: "high",
          title: i18n.t("permissions:grant.privateNetwork.title"),
          detail: i18n.t("permissions:grant.privateNetwork.detail"),
        }
      }
      return {
        tier: "medium",
        title: i18n.t("permissions:grant.network.title", { host: describeHost(target) }),
        detail: i18n.t("permissions:grant.network.detail", { methods: methodsText(facts.methods) }),
      }
    case "credentials":
      return {
        tier: "high",
        title: i18n.t("permissions:grant.credentials.title", { secret: target }),
        detail: i18n.t("permissions:grant.credentials.detail"),
      }
    case "notifications":
      return {
        tier: "low",
        title: i18n.t("permissions:grant.notifications.title"),
        detail: facts.routed
          ? i18n.t("permissions:grant.notifications.detailRouted")
          : i18n.t("permissions:grant.notifications.detail"),
      }
    case "profile":
      return {
        tier: "medium",
        title: profileTitle(facts.read),
        detail: i18n.t("permissions:grant.profile.detail"),
      }
    case "variables":
      return {
        tier: "medium",
        title: target.endsWith(".*")
          ? i18n.t("permissions:grant.variables.titlePrefix", { prefix: target.slice(0, -2) })
          : i18n.t("permissions:grant.variables.title", { name: target }),
        detail: i18n.t("permissions:grant.variables.detail"),
      }
    case "storage":
      return {
        tier: "low",
        title:
          facts.quotaKb === undefined
            ? i18n.t("permissions:grant.storage.title")
            : i18n.t("permissions:grant.storage.titleQuota", { count: facts.quotaKb }),
        detail: i18n.t("permissions:grant.storage.detail"),
      }
    case "openUrl":
      if (key.startsWith(OPEN_USER_ORIGIN_PREFIX)) {
        const setting = key.slice(OPEN_USER_ORIGIN_PREFIX.length)
        return {
          tier: "medium",
          title: i18n.t("permissions:grant.openUserOrigin.title", { setting }),
          detail: i18n.t("permissions:grant.openUserOrigin.detail"),
        }
      }
      return {
        tier: "medium",
        title: i18n.t("permissions:grant.openUrl.title", { host: describeHost(target) }),
        detail: i18n.t("permissions:grant.openUrl.detail"),
      }
    case "background":
      return {
        tier: "medium",
        title: i18n.t("permissions:grant.background.title"),
        detail:
          facts.minIntervalSeconds === undefined
            ? i18n.t("permissions:grant.background.detail")
            : i18n.t("permissions:grant.background.detailInterval", {
                count: facts.minIntervalSeconds,
              }),
      }
    case "device":
      return {
        tier: "low",
        title:
          target === "icons"
            ? i18n.t("permissions:grant.device.titleIcons")
            : i18n.t("permissions:grant.device.titleState"),
        detail: i18n.t("permissions:grant.device.detail"),
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
        for (const slot of permission.userOrigins ?? []) {
          const methods = slot.methods ?? permission.methods
          add(`${USER_ORIGIN_PREFIX}${slot.setting}`, justification, {
            ...(methods ? { methods } : {}),
            pathPrefix: slot.pathPrefix === true,
            privateNetwork: slot.privateNetwork === true,
          })
        }
        break
      case "credentials":
        for (const ref of permission.refs) add(`credentials:${ref}`, justification)
        break
      case "notifications":
        add("notifications", justification, permission.routing ? { routed: true } : {})
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
        for (const setting of permission.userOrigins ?? []) {
          add(`${OPEN_USER_ORIGIN_PREFIX}${setting}`, justification)
        }
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
  if (s.pathPrefix === true) facts.pathPrefix = true
  if (s.privateNetwork === true) facts.privateNetwork = true
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
  return tiers()
    .map((tier) => ({
      ...tier,
      grants: grants.filter((g) => g.tier === tier.tier),
    }))
    .filter((group) => group.grants.length > 0)
}
