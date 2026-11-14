// Version and platform compatibility of plugin versions with this DeckPilot.
//
// For display only: the agent decides eligibility (architecture §12.3 step 2).

import type {
  PluginEnvironment,
  PluginPlatform,
  RegistryPluginVersion,
  RegistryRevocation,
} from "./types"

export const PLATFORM_LABELS: Record<PluginPlatform, string> = {
  macos: "macOS",
  windows: "Windows",
  linux: "Linux",
}

const ALL_PLATFORMS: PluginPlatform[] = ["macos", "windows", "linux"]

/** Platforms of a version; omitted means all. */
export function versionPlatforms(version: RegistryPluginVersion): PluginPlatform[] {
  return version.platforms ?? ALL_PLATFORMS
}

export function platformList(platforms: PluginPlatform[]): string {
  return platforms.map((p) => PLATFORM_LABELS[p]).join(", ")
}

interface Semver {
  core: [number, number, number]
  /** alpha < beta < rc < release. */
  pre: [number, number] | null
}

const PRE_RANK = { alpha: 0, beta: 1, rc: 2 } as const

/** The release-tag grammar of the manifest schema (`$defs/semver`). */
export function parseSemver(value: string): Semver | null {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(alpha|beta|rc)\.([1-9]\d*))?$/.exec(
    value,
  )
  if (!match) return null
  const [, major, minor, patch, tag, n] = match
  return {
    core: [Number(major), Number(minor), Number(patch)],
    pre: tag ? [PRE_RANK[tag as keyof typeof PRE_RANK], Number(n)] : null,
  }
}

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
  return x.pre[0] - y.pre[0] || x.pre[1] - y.pre[1]
}

export function isPrerelease(version: string): boolean {
  return parseSemver(version)?.pre != null
}

/** `engines.deckpilot`: one or two comparators separated by a space. */
export function satisfiesRange(version: string, range: string): boolean {
  const comparators = range.trim().split(/\s+/)
  return comparators.every((comparator) => {
    const match = /^(>=|>|<=|<|=)?(.+)$/.exec(comparator)
    if (!match) return false
    const op = match[1] ?? "="
    const target = match[2] ?? ""
    if (!parseSemver(target)) return false
    const d = compareSemver(version, target)
    switch (op) {
      case ">=":
        return d >= 0
      case ">":
        return d > 0
      case "<=":
        return d <= 0
      case "<":
        return d < 0
      default:
        return d === 0
    }
  })
}

/** `engines.pluginApi` "^M.m": same major, at least that minor. */
export function satisfiesApi(provided: string, required: string): boolean {
  const need = /^\^(\d+)\.(\d+)$/.exec(required)
  const have = /^(\d+)\.(\d+)$/.exec(provided)
  if (!need || !have) return false
  return need[1] === have[1] && Number(have[2]) >= Number(need[2])
}

export interface Compatibility {
  compatible: boolean
  /** Why not, one sentence each. */
  reasons: string[]
}

export function compatibility(
  version: RegistryPluginVersion,
  environment: PluginEnvironment,
): Compatibility {
  const reasons: string[] = []
  if (!environment.pluginsSupported) {
    reasons.push(`Plugins are not available on ${PLATFORM_LABELS[environment.platform]}.`)
  }
  if (!satisfiesRange(environment.appVersion, version.engines.deckpilot)) {
    reasons.push(
      `Requires DeckPilot ${version.engines.deckpilot}; this is DeckPilot ${environment.appVersion}.`,
    )
  }
  if (!satisfiesApi(environment.pluginApi, version.engines.pluginApi)) {
    reasons.push(
      `Requires plugin API ${version.engines.pluginApi}; this DeckPilot provides ${environment.pluginApi}.`,
    )
  }
  const platforms = versionPlatforms(version)
  if (!platforms.includes(environment.platform)) {
    reasons.push(
      `Not available for ${PLATFORM_LABELS[environment.platform]}; supports ${platformList(platforms)}.`,
    )
  }
  return { compatible: reasons.length === 0, reasons }
}

/** The revocation of the registry that covers `version`, `disable` first. */
export function revocationOf(
  revocations: RegistryRevocation[],
  version: string,
): RegistryRevocation | undefined {
  const matching = revocations.filter((r) => r.versions === "*" || r.versions.includes(version))
  return matching.find((r) => r.action === "disable") ?? matching[0]
}

export interface OfferedVersion {
  version: RegistryPluginVersion
  compatibility: Compatibility
}

/**
 * The version Discover offers: the newest stable one that is neither yanked
 * nor revoked, preferring compatible versions. `null` when nothing can be
 * offered (every version yanked or revoked).
 */
export function offeredVersion(
  versions: RegistryPluginVersion[],
  revocations: RegistryRevocation[],
  environment: PluginEnvironment,
): OfferedVersion | null {
  const candidates = versions
    .filter((v) => !v.yanked && !revocationOf(revocations, v.version) && !isPrerelease(v.version))
    .sort((a, b) => compareSemver(b.version, a.version))
  const rated = candidates.map((version) => ({
    version,
    compatibility: compatibility(version, environment),
  }))
  return rated.find((r) => r.compatibility.compatible) ?? rated[0] ?? null
}
