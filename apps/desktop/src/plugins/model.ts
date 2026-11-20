// Pure helpers of the marketplace screens.

import { type TranslationKey, currentLanguage, i18n } from "@/i18n"
import { formatDate as formatLocalDate, formatDateTime } from "@/i18n/format"

import { toPluginError, type PluginClient } from "./api"
import type {
  AvailablePlugin,
  AvailableVersion,
  KnownPlugin,
  MarketplaceSnapshot,
  PendingPluginUpdate,
  PluginDetails,
  PluginRuntimeStatus,
  PluginState,
  PluginStorageUsage,
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
  const missing = await missingSettings(client, installed)
  if (Object.keys(missing).length > 0) snapshot = { ...snapshot, missingSettings: missing }
  try {
    snapshot = { ...snapshot, known: await client.listKnownPlugins() }
  } catch {
    // Only names for messages; the marketplace works without them.
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

/**
 * The required settings each plugin lacks. Only plugins whose manifest has
 * a required setting without a default can lack one, so only they are
 * asked; a refusal leaves the plugin out.
 */
async function missingSettings(
  client: PluginClient,
  installed: PluginDetails[],
): Promise<Record<string, string[]>> {
  const out: Record<string, string[]> = {}
  await Promise.all(
    installed
      .filter((p) => declaredConfiguration(p.manifest).settings.some((f) => f.mayBeMissing))
      .map(async (p) => {
        try {
          const settings = await client.getSettings(p.summary.id)
          const labels = (settings.missing ?? []).map(
            (key) => settings.fields.find((f) => f.key === key)?.label ?? key,
          )
          if (labels.length > 0) out[p.summary.id] = labels
        } catch {
          // Shown when the user opens Configure.
        }
      }),
  )
  return out
}

/** What `plugin_known` has for a plugin that is not installed now. */
export function knownUninstalled(
  snapshot: MarketplaceSnapshot,
  id: string,
): KnownPlugin | undefined {
  return snapshot.known?.find((k) => k.id === id && !k.installed)
}

/** One decimal below 10, none above, in the window's number format (`1.5` / `1,5`). */
function oneDecimal(n: number): string {
  return new Intl.NumberFormat(currentLanguage(), {
    maximumFractionDigits: n < 10 ? 1 : 0,
    useGrouping: false,
  }).format(n)
}

/** `512 B`, `1.5 KiB`, `256 KiB`, `2 MiB`. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${String(bytes)} B`
  const kib = bytes / 1024
  if (kib < 1024) return `${oneDecimal(kib)} KiB`
  return `${oneDecimal(kib / 1024)} MiB`
}

/** What a plugin's storage holds, against its quota. */
export function storageText(usage: PluginStorageUsage): string {
  return i18n.t("plugins:storage.usage", {
    used: formatBytes(usage.usedBytes),
    quota: formatBytes(usage.quotaBytes),
    keys: i18n.t("plugins:storage.keys", { count: usage.maxKeys, used: String(usage.keys) }),
  })
}

/**
 * The kinds of functionality the marketplace distinguishes, from data it
 * already has: the built-in modules (core functionality, not plugins),
 * plugins that ship with DeckPilot (origin `bundled`), optional plugins the user
 * installed, and registry plugins that are not installed.
 */
export interface PluginClasses {
  core: { id: string; name: string }[]
  bundled: PluginDetails[]
  installed: PluginDetails[]
  available: AvailablePlugin[]
}

/**
 * Whether a plugin ships with DeckPilot (origin `bundled`: a first-party
 * plugin checked against the pins compiled into the agent). The screens
 * show these apart from the optional plugins the user installed.
 */
export function isBundledPlugin(plugin: PluginDetails): boolean {
  return plugin.summary.origin.type === "bundled"
}

/** The longest secret the agent accepts (`MAX_PLUGIN_SECRET`), in UTF-8 bytes. */
export const MAX_SECRET_BYTES = 16 * 1024

/** Why `secret` cannot be sent to the agent; never repeats the secret. */
export function secretProblem(secret: string): string | null {
  if (secret === "") return i18n.t("plugins:credentials.problem.empty")
  if (/[\r\n\0]/.test(secret)) return i18n.t("plugins:credentials.problem.lineBreaks")
  if (new TextEncoder().encode(secret).length > MAX_SECRET_BYTES) {
    return i18n.t("plugins:credentials.problem.tooLong")
  }
  return null
}

export function classifyPlugins(
  snapshot: MarketplaceSnapshot,
  modules: { id: string; name: string }[],
): PluginClasses {
  const installedIds = new Set(snapshot.installed.map((p) => p.summary.id))
  return {
    // Built-in module ids never contain a `.`; plugin ids always do.
    core: modules
      .filter((m) => !m.id.includes("."))
      .map((m) => ({ id: m.id, name: m.name }))
      .sort((a, b) => a.name.localeCompare(b.name)),
    bundled: snapshot.installed.filter(isBundledPlugin),
    installed: snapshot.installed.filter((p) => !isBundledPlugin(p)),
    available: snapshot.available.filter((p) => !installedIds.has(p.id) && !p.installed),
  }
}

/** Why nothing can be installed or updated right now, or `null`. */
export function installBlock(snapshot: MarketplaceSnapshot): string | null {
  if (!snapshot.registry) {
    return i18n.t("marketplace:blocked.noRegistry")
  }
  switch (snapshot.registry.freshness) {
    case "fresh":
      return null
    case "stale":
      return i18n.t("marketplace:blocked.stale")
    case "neverVerified":
      return i18n.t("marketplace:blocked.neverVerified")
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

const STATE_LABELS = {
  installing: "plugins:state.installing",
  disabled: "plugins:state.disabled",
  enabled: "plugins:state.enabled",
  updating: "plugins:state.updating",
  rollingBack: "plugins:state.rollingBack",
  uninstalling: "plugins:state.uninstalling",
  broken: "plugins:state.broken",
  quarantined: "plugins:state.quarantined",
} as const satisfies Record<PluginState, TranslationKey>

/** A plugin's state as a label ("Enabled"). */
export function stateLabel(state: PluginState): string {
  return i18n.t(STATE_LABELS[state])
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

/** "A, B and C" in the window's language (without a serial comma). */
export function listed(items: string[]): string {
  if (items.length <= 1) return items.join("")
  return i18n.t("plugins:list.and", {
    items: items.slice(0, -1).join(", "),
    last: items[items.length - 1] ?? "",
  })
}

/** Platforms of a version in words; none listed means every platform. */
export function platformsText(platforms: string[] | undefined): string {
  if (!platforms || platforms.length === 0) return i18n.t("marketplace:platforms.all")
  const labels = platforms.map(platformLabel)
  if (labels.length === 1) return i18n.t("marketplace:platforms.only", { platform: labels[0] })
  return listed(labels)
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
  return formatLocalDate(currentLanguage(), seconds * 1000, {
    year: "numeric",
    month: "short",
    day: "numeric",
  })
}

/** A time of the agent (seconds since the epoch) for people. */
export function formatTime(seconds: number): string {
  return formatDateTime(currentLanguage(), seconds * 1000)
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
  /** A required setting without a default: unset until the user sets it. */
  mayBeMissing?: boolean
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
              ...(r.required === true && r.default === undefined ? { mayBeMissing: true } : {}),
            },
          ]
        })
      : []
  return {
    settings: fields(record(contributes?.settings)?.fields),
    credentials: fields(contributes?.credentials),
  }
}
