// Pure helpers of the marketplace screens.

import { toPluginError, type PluginClient } from "./api"
import type {
  AvailablePlugin,
  AvailableVersion,
  MarketplaceSnapshot,
  PendingPluginUpdate,
  PluginDetails,
  PluginRuntimeStatus,
  PluginState,
  RuntimeAvailability,
} from "./types"

/** Whether and how plugins can be used, from the runtime status. */
export function availabilityOf(status: PluginRuntimeStatus): RuntimeAvailability {
  if (status.available) {
    return status.executionSupported
      ? { kind: "ready", status }
      : { kind: "platformUnsupported", status }
  }
  if (!status.built) return { kind: "notBuilt", status }
  if (!status.enabled) return { kind: "notEnabled", status }
  return { kind: "unavailable", status }
}

/** The availability of a `getPluginRuntime` refusal (no agent, an old agent). */
export function availabilityOfError(error: unknown): RuntimeAvailability | null {
  const e = toPluginError(error)
  if (e.reason === "agentNotRunning") return { kind: "agentNotRunning", message: e.message }
  if (e.reason === "agentTooOld") return { kind: "agentTooOld", message: e.message }
  return null
}

/**
 * Everything the screens show. Installed plugins are read in full (their
 * permissions and pending updates); a registry that cannot be listed is
 * reported next to them rather than failing the page.
 */
export async function loadSnapshot(
  client: PluginClient,
  runtime: PluginRuntimeStatus,
  refresh = false,
): Promise<MarketplaceSnapshot> {
  const summaries = await client.listPlugins()
  const installed = await Promise.all(summaries.map((s) => client.getPlugin(s.id)))
  let snapshot: MarketplaceSnapshot = {
    runtime,
    registry: runtime.registry ?? null,
    available: [],
    installed,
  }
  try {
    const available = await client.listAvailablePlugins(refresh)
    snapshot = { ...snapshot, registry: available.registry, available: available.plugins }
  } catch (error) {
    const e = toPluginError(error)
    snapshot = { ...snapshot, registryError: { reason: e.reason, message: e.message } }
  }
  return snapshot
}

/** Why nothing can be installed or updated right now, or `null`. */
export function installBlock(snapshot: MarketplaceSnapshot): string | null {
  if (!snapshot.registry) {
    return "No plugin registry is configured in the agent (--plugin-registry)."
  }
  switch (snapshot.registry.freshness) {
    case "fresh":
      return null
    case "stale":
      return "The registry is out of date; installing and updating are paused until it is checked again."
    case "neverVerified":
      return "The registry has not been verified yet; nothing can be installed."
  }
}

interface Semver {
  core: [number, number, number]
  pre: string[] | null
}

function parseSemver(value: string): Semver | null {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+.*)?$/.exec(value)
  if (!match) return null
  const [, major, minor, patch, pre] = match
  return {
    core: [Number(major), Number(minor), Number(patch)],
    pre: pre ? pre.split(".") : null,
  }
}

/** SemVer precedence; unparsable versions compare as text. */
export function compareSemver(a: string, b: string): number {
  const x = parseSemver(a)
  const y = parseSemver(b)
  if (!x || !y) return a.localeCompare(b)
  for (let i = 0; i < 3; i++) {
    const d = (x.core[i] ?? 0) - (y.core[i] ?? 0)
    if (d !== 0) return d
  }
  if (!x.pre && !y.pre) return 0
  if (!x.pre) return 1
  if (!y.pre) return -1
  for (let i = 0; i < Math.max(x.pre.length, y.pre.length); i++) {
    const p = x.pre[i]
    const q = y.pre[i]
    if (p === undefined) return -1
    if (q === undefined) return 1
    const np = /^\d+$/.test(p)
    const nq = /^\d+$/.test(q)
    if (np && nq && Number(p) !== Number(q)) return Number(p) - Number(q)
    if (np !== nq) return np ? -1 : 1
    if (p !== q) return p.localeCompare(q)
  }
  return 0
}

export function isInstallable(version: AvailableVersion): boolean {
  return !version.withdrawn && (version.incompatible ?? []).length === 0
}

/**
 * The version Discover offers: the registry's latest when installable,
 * otherwise the newest installable one. `null` when none can be installed.
 */
export function offeredVersion(plugin: AvailablePlugin): AvailableVersion | null {
  const latest = plugin.versions.find((v) => v.version === plugin.latest)
  if (latest && isInstallable(latest)) return latest
  return (
    [...plugin.versions]
      .sort((a, b) => compareSemver(b.version, a.version))
      .find((v) => isInstallable(v)) ?? null
  )
}

/** A newer version of an installed plugin. */
export type UpdateCandidate =
  /** Staged by the agent, waiting for consent to its escalation. */
  | { kind: "staged"; plugin: PluginDetails; pending: PendingPluginUpdate }
  /** Offered by the registry; the agent decides whether it needs consent. */
  | { kind: "available"; plugin: PluginDetails; version: string }

export function updateCandidates(snapshot: MarketplaceSnapshot): UpdateCandidate[] {
  return snapshot.installed.flatMap((plugin): UpdateCandidate[] => {
    if (plugin.pendingUpdate) return [{ kind: "staged", plugin, pending: plugin.pendingUpdate }]
    const offered = snapshot.available.find((a) => a.id === plugin.summary.id)
    const version = offered && offeredVersion(offered)
    if (version && compareSemver(version.version, plugin.summary.version) > 0) {
      return [{ kind: "available", plugin, version: version.version }]
    }
    return []
  })
}

export const STATE_LABELS: Record<PluginState, string> = {
  installing: "Installing",
  disabled: "Disabled",
  enabled: "Enabled",
  updating: "Updating",
  rollingBack: "Rolling back",
  uninstalling: "Uninstalling",
  broken: "Broken",
  quarantined: "Quarantined",
}

/** Entries whose name, id, summary or publisher contain every word. */
export function matchesQuery(item: AvailablePlugin, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  const text = [item.name, item.id, item.summary, item.publisher, item.publisherInfo?.displayName]
    .filter(Boolean)
    .join(" ")
    .toLowerCase()
  return words.every((word) => text.includes(word))
}

/** Entries in `category`; `null` is every category. */
export function inCategory(item: AvailablePlugin, category: string | null): boolean {
  return category === null || (item.categories ?? []).includes(category)
}

export const PLATFORM_LABELS: Record<string, string> = {
  macos: "macOS",
  windows: "Windows",
  linux: "Linux",
}

export function platformLabel(platform: string): string {
  return PLATFORM_LABELS[platform] ?? platform
}

/** Platforms of a version in words; none listed means every platform. */
export function platformsText(platforms: string[] | undefined): string {
  if (!platforms || platforms.length === 0) return "macOS, Windows and Linux"
  const labels = platforms.map(platformLabel)
  if (labels.length === 1) return `${labels[0] ?? ""} only`
  return `${labels.slice(0, -1).join(", ")} and ${labels.at(-1) ?? ""}`
}

/** Every category the registry's plugins name, sorted. */
export function categoriesOf(plugins: AvailablePlugin[]): string[] {
  return [...new Set(plugins.flatMap((p) => p.categories ?? []))].sort((a, b) => a.localeCompare(b))
}

/** A category id (`developer-tools`) in words (`Developer tools`). */
export function categoryLabel(category: string): string {
  const text = category.replace(/[-_]+/g, " ")
  return text.charAt(0).toUpperCase() + text.slice(1)
}

/** A date of the agent (seconds since the epoch) for people. */
export function formatDate(seconds: number): string {
  return new Date(seconds * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  })
}

/** A time of the agent (seconds since the epoch) for people. */
export function formatTime(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  })
}

function record(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : undefined
}

/** What an installed version's manifest says it needs (`engines`, `platforms`). */
export function manifestCompatibility(manifest: unknown): {
  deckpilot?: string
  platforms?: string[]
} {
  const m = record(manifest)
  const deckpilot = record(m?.engines)?.deckpilot
  const platforms = Array.isArray(m?.platforms)
    ? m.platforms.filter((p): p is string => typeof p === "string")
    : undefined
  return {
    ...(typeof deckpilot === "string" ? { deckpilot } : {}),
    ...(platforms ? { platforms } : {}),
  }
}

/** A setting or credential slot an installed version declares. */
export interface DeclaredField {
  key: string
  label: string
  description?: string
}

/** The settings and credential slots of a manifest (`contributes`). */
export function declaredConfiguration(manifest: unknown): {
  settings: DeclaredField[]
  credentials: DeclaredField[]
} {
  const contributes = record(record(manifest)?.contributes)
  const fields = (list: unknown): DeclaredField[] =>
    Array.isArray(list)
      ? list.flatMap((item): DeclaredField[] => {
          const r = record(item)
          if (typeof r?.key !== "string") return []
          return [
            {
              key: r.key,
              label: typeof r.label === "string" ? r.label : r.key,
              ...(typeof r.description === "string" ? { description: r.description } : {}),
            },
          ]
        })
      : []
  return {
    settings: fields(record(contributes?.settings)?.fields),
    credentials: fields(contributes?.credentials),
  }
}
