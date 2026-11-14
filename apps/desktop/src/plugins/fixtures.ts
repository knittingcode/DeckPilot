// Development fixtures for the experimental marketplace UI.
//
// NOT live marketplace data: publishers, plugins, hashes and commits are
// made up, and every screen that uses this client says so. The client keeps
// its state in memory and simulates the refusals the agent is designed to
// make (architecture §12.3, §13.2, §14), so the UI's states can be reviewed
// before the backend exists.

import { PluginError, type InstallRequest, type PluginClient } from "./api"
import { compareSemver, compatibility, offeredVersion, revocationOf } from "./compat"
import { compareGrants, grantsOf } from "./permissions"
import type {
  CatalogEntry,
  InstalledPlugin,
  MarketplaceSnapshot,
  PluginDetail,
  PluginEnvironment,
  PluginPermission,
  RegistryIndexEntry,
  RegistryPlugin,
  RegistryPluginVersion,
  RegistryPublisher,
  RegistryRevocation,
  RegistryState,
  StoredGrant,
} from "./types"

export const FIXTURE_LABEL = "Development fixtures — not live marketplace data"

const NOW = "2026-11-14T09:00:00Z"
const HASH = "0".repeat(63)
const COMMIT = "0".repeat(39)

const publishers: RegistryPublisher[] = [
  {
    namespace: "acme",
    github_id: 1001,
    login: "fixture-acme",
    display_name: "Acme Fixtures",
    verified: true,
  },
  {
    namespace: "orbit",
    github_id: 1002,
    login: "fixture-orbit",
    display_name: "Orbit Fixtures",
    verified: true,
  },
  { namespace: "tempo", github_id: 1003, login: "fixture-tempo", verified: false },
  { namespace: "knit", github_id: 1004, login: "fixture-knit", verified: false },
  { namespace: "legacy", github_id: 1005, login: "fixture-legacy", verified: true },
  { namespace: "shady", github_id: 1006, login: "fixture-shady", verified: false },
  { namespace: "beta", github_id: 1007, login: "fixture-beta", verified: false },
  { namespace: "cal", github_id: 1008, login: "fixture-cal", verified: false },
]

function version(
  id: string,
  value: string,
  permissions: PluginPermission[],
  extra: Partial<RegistryPluginVersion> = {},
): RegistryPluginVersion {
  const [publisher = "", name = ""] = id.split(".")
  return {
    version: value,
    published_at: NOW,
    engines: { deckpilot: ">=0.6.0 <1.0.0", pluginApi: "^1.0" },
    permissions,
    manifest_sha256: `${HASH}${String(value.length % 10)}`,
    source: {
      repository: `fixture-${publisher}/deckpilot-${name}`,
      tag: `v${value}`,
      commit: `${COMMIT}${String(value.length % 10)}`,
    },
    package: {
      url: `https://github.com/knittingcode/deckpilot-plugins/releases/download/${id}--v${value}/${id}.dpplugin`,
      sha256: `${HASH}1`,
      size: 48_213,
    },
    builder: "1.0.0",
    ...extra,
  }
}

interface FixturePlugin {
  plugin: RegistryPlugin
  entry: Omit<RegistryIndexEntry, "id" | "publisher" | "name" | "detail">
}

function plugin(
  id: string,
  name: string,
  summary: string,
  description: string,
  categories: RegistryIndexEntry["categories"],
  versions: RegistryPluginVersion[],
  extra: Partial<RegistryIndexEntry> = {},
): FixturePlugin {
  const publisher = id.split(".")[0] ?? ""
  const stable = [...versions].reverse().find((v) => !v.version.includes("-"))?.version
  return {
    plugin: {
      _type: "deckpilot-registry-plugin",
      spec_version: 1,
      id,
      publisher,
      name,
      description,
      license: "MIT",
      source_repository: versions[0]?.source.repository,
      versions,
    },
    entry: { summary, categories, latest: { stable }, ...extra },
  }
}

const githubV1: PluginPermission[] = [
  {
    kind: "network",
    hosts: ["api.github.com"],
    methods: ["GET"],
    justification: "Reads the pull requests that wait for your review.",
  },
  {
    kind: "credentials",
    refs: ["token"],
    justification: "Authenticates the requests to the GitHub API.",
  },
  {
    kind: "background",
    minIntervalSeconds: 60,
    justification: "Refreshes the review count on the key every minute.",
  },
  { kind: "notifications", justification: "Tells you when a new review request arrives." },
]
const githubV2: PluginPermission[] = [
  ...githubV1,
  {
    kind: "openUrl",
    hosts: ["github.com"],
    justification: "Opens a pull request when you press its key.",
  },
]
const statusV1: PluginPermission[] = [
  {
    kind: "network",
    hosts: ["*.statuspage.io"],
    methods: ["GET"],
    justification: "Reads the status of the pages you choose.",
  },
  { kind: "notifications", justification: "Tells you when a component changes its status." },
  { kind: "storage", quotaKb: 512, justification: "Keeps the history of incidents for the key." },
]
const statusV2 = statusV1.filter((p) => p.kind !== "storage")
const timer: PluginPermission[] = [
  {
    kind: "device",
    state: true,
    icons: true,
    justification: "Shows the remaining time on the key.",
  },
  { kind: "notifications", justification: "Tells you when a focus session ends." },
]
const lights: PluginPermission[] = [
  {
    kind: "network",
    hosts: ["api.lifx.com"],
    methods: ["GET", "POST"],
    privateNetwork: true,
    justification: "Switches lights through the cloud API or your local bridge.",
  },
  {
    kind: "credentials",
    refs: ["lifxToken"],
    justification: "Authenticates the requests to the LIFX cloud API.",
  },
  { kind: "device", state: true, justification: "Shows whether a light is on." },
]
const deploy: PluginPermission[] = [
  {
    kind: "network",
    hosts: ["deploy.example.com"],
    methods: ["POST"],
    justification: "Starts deployments of the project you choose.",
  },
  {
    kind: "profile",
    read: ["project"],
    justification: "Picks the deployment target of the active project.",
  },
]

const PLUGINS: FixturePlugin[] = [
  plugin(
    "acme.github-prs",
    "GitHub Pull Requests",
    "Shows how many pull requests wait for your review.",
    "Counts the pull requests that request your review and opens them from a key.",
    ["developer-tools"],
    [version("acme.github-prs", "1.2.0", githubV1), version("acme.github-prs", "1.3.0", githubV2)],
  ),
  plugin(
    "orbit.status-page",
    "Status Page Monitor",
    "Watches Statuspage components and alerts on incidents.",
    "Colours a key by the status of the components you pick on any statuspage.io page.",
    ["monitoring"],
    [
      version("orbit.status-page", "2.0.0", statusV1, {
        yanked: { at: NOW, reason: "Notifications repeat every minute." },
      }),
      version("orbit.status-page", "2.0.1", statusV2),
    ],
  ),
  plugin(
    "tempo.focus-timer",
    "Focus Timer",
    "A Pomodoro timer on a key.",
    "Starts focus sessions and breaks and shows the remaining time on the key.",
    ["productivity"],
    [version("tempo.focus-timer", "1.0.0", timer)],
  ),
  plugin(
    "knit.home-lights",
    "Home Lights",
    "Switch LIFX lights from your Stream Deck.",
    "Turns lights on and off and shows their state, through the LIFX cloud or a bridge on your network.",
    ["smart-home"],
    [version("knit.home-lights", "0.3.0", lights)],
  ),
  plugin(
    "legacy.weather-keys",
    "Weather Keys",
    "The local forecast on a key.",
    "Shows the temperature and forecast for a city you choose.",
    ["other"],
    [
      version("legacy.weather-keys", "1.0.0", [], {
        engines: { deckpilot: ">=0.9.0", pluginApi: "^1.2" },
        platforms: ["windows"],
      }),
    ],
  ),
  plugin(
    "shady.clipboard-sync",
    "Clipboard Sync",
    "Shares snippets between computers.",
    "Sends text snippets to your other computers.",
    ["productivity"],
    [
      version("shady.clipboard-sync", "1.0.1", [
        {
          kind: "network",
          hosts: ["sync.example.net"],
          justification: "Sends snippets to your other computers.",
        },
      ]),
    ],
  ),
  plugin(
    "beta.chat-bridge",
    "Chat Bridge",
    "Post canned messages to a chat webhook.",
    "Sends a message you prepared to a chat webhook when you press a key.",
    ["communication"],
    [
      version("beta.chat-bridge", "1.0.0", [
        {
          kind: "network",
          hosts: ["hooks.example.org"],
          methods: ["POST"],
          justification: "Posts the message to the webhook.",
        },
      ]),
      version("beta.chat-bridge", "1.1.0", [], {
        yanked: { at: NOW, reason: "Crashes on start." },
      }),
    ],
  ),
  plugin(
    "acme.deploy-buttons",
    "Deploy Buttons",
    "Start deployments from a key.",
    "Starts a deployment of the active project and shows its progress.",
    ["developer-tools"],
    [
      version("acme.deploy-buttons", "0.8.2", deploy),
      version("acme.deploy-buttons", "0.9.0", deploy),
    ],
  ),
  plugin(
    "cal.calendar-keys",
    "Calendar Keys",
    "Your next meeting on a key.",
    "Shows the time until your next meeting.",
    ["productivity"],
    [
      version("cal.calendar-keys", "1.0.0", [
        {
          kind: "network",
          hosts: ["calendar.example.com"],
          methods: ["GET"],
          justification: "Reads your upcoming events.",
        },
      ]),
    ],
  ),
  plugin(
    "knit.old-clock",
    "Old Clock",
    "A clock on a key.",
    "Shows the time.",
    ["other"],
    [version("knit.old-clock", "1.0.0", [])],
    {
      deprecated: { at: NOW, reason: "Replaced by Focus Timer.", replacement: "tempo.focus-timer" },
    },
  ),
]

const REVOCATIONS: RegistryRevocation[] = [
  {
    id: "shady.clipboard-sync",
    versions: "*",
    action: "disable",
    reason: "Sent clipboard contents to a host its manifest does not declare.",
    advisory: "https://github.com/knittingcode/deckpilot-plugins/security/advisories",
    at: NOW,
  },
]

/** Fixture ids whose package fails verification when installed. */
const BAD_PACKAGES = new Set(["cal.calendar-keys"])

function stored(
  permissions: PluginPermission[],
  version: string,
  revoked: string[] = [],
): StoredGrant[] {
  return grantsOf(permissions).map((grant) => ({
    key: grant.key,
    permission: grant.permission,
    enabled: !revoked.includes(grant.key),
    grantedAt: NOW,
    grantedForVersion: version,
    source: "install",
  }))
}

function installedFixtures(): InstalledPlugin[] {
  return [
    {
      id: "acme.github-prs",
      name: "GitHub Pull Requests",
      publisher: "acme",
      version: "1.2.0",
      source: "registry",
      enabled: true,
      permissions: githubV1,
      grants: stored(githubV1, "1.2.0"),
      installedAt: NOW,
      problems: [],
    },
    {
      id: "orbit.status-page",
      name: "Status Page Monitor",
      publisher: "orbit",
      version: "2.0.0",
      source: "registry",
      enabled: true,
      permissions: statusV1,
      grants: stored(statusV1, "2.0.0"),
      installedAt: NOW,
      problems: [{ kind: "yanked", reason: "Notifications repeat every minute." }],
    },
    {
      id: "tempo.focus-timer",
      name: "Focus Timer",
      publisher: "tempo",
      version: "1.0.0",
      source: "registry",
      enabled: false,
      permissions: timer,
      grants: stored(timer, "1.0.0", ["notifications"]),
      installedAt: NOW,
      problems: [],
    },
    {
      id: "shady.clipboard-sync",
      name: "Clipboard Sync",
      publisher: "shady",
      version: "1.0.1",
      source: "registry",
      enabled: false,
      permissions: PLUGINS[5]?.plugin.versions[0]?.permissions ?? [],
      grants: stored(PLUGINS[5]?.plugin.versions[0]?.permissions ?? [], "1.0.1"),
      installedAt: NOW,
      problems: [
        {
          kind: "revoked",
          reason: "Sent clipboard contents to a host its manifest does not declare.",
          advisory: "https://github.com/knittingcode/deckpilot-plugins/security/advisories",
        },
      ],
    },
    {
      id: "acme.deploy-buttons",
      name: "Deploy Buttons",
      publisher: "acme",
      version: "0.9.0",
      source: "registry",
      enabled: true,
      permissions: deploy,
      grants: stored(deploy, "0.9.0"),
      installedAt: NOW,
      previousVersion: "0.8.2",
      problems: [
        {
          kind: "broken",
          code: "registration-failed",
          message: "The action type deploy-buttons.run is registered twice.",
          hint: "Roll back to the previous version or uninstall the plugin, and report the problem to its publisher.",
        },
      ],
    },
  ]
}

export interface FixtureOptions {
  registry?: RegistryState
  /** No plugins in the registry and none installed. */
  empty?: boolean
  /** `listPlugins` fails, as if the agent did not answer. */
  failList?: boolean
  environment?: Partial<PluginEnvironment>
  /** Simulated latency of every request. */
  delayMs?: number
}

export interface FixtureClient extends PluginClient {
  /** Switches the simulated registry state (development only). */
  simulate(registry: RegistryState): void
  readonly registryState: RegistryState
}

export const DEFAULT_FIXTURE_ENVIRONMENT: PluginEnvironment = {
  appVersion: "0.6.0",
  platform: "macos",
  pluginApi: "1.0",
  pluginsSupported: true,
}

const REGISTRY_MESSAGES: Partial<Record<RegistryState, string>> = {
  offline: "The registry could not be reached: no network connection.",
  stale: "The registry's signed timestamp expired more than 7 days ago.",
  "verification-failed": "The registry index signature did not verify against the trusted keys.",
  "never-reached": "This computer has never reached the registry.",
}

/** An in-memory client with made-up data; see the file comment. */
export function createFixtureClient(options: FixtureOptions = {}): FixtureClient {
  const environment = { ...DEFAULT_FIXTURE_ENVIRONMENT, ...options.environment }
  let registry: RegistryState = options.registry ?? "fresh"
  let checkedAt = NOW
  const plugins = options.empty ? [] : PLUGINS
  let installed = options.empty ? [] : installedFixtures()

  const reply = <T>(make: () => T): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      const run = () => {
        try {
          resolve(structuredClone(make()))
        } catch (error) {
          reject(error instanceof Error ? error : new Error(String(error)))
        }
      }
      if (options.delayMs) setTimeout(run, options.delayMs)
      else queueMicrotask(run)
    })

  const find = (id: string) => {
    const found = plugins.find((p) => p.plugin.id === id)
    if (!found) throw new PluginError("not-found", `The registry has no plugin ${id}.`)
    return found
  }
  const revocations = (id: string) => REVOCATIONS.filter((r) => r.id === id)
  const catalog = (): CatalogEntry[] =>
    registry === "never-reached"
      ? []
      : plugins.map(({ plugin: p, entry }) => ({
          entry: {
            ...entry,
            id: p.id,
            publisher: p.publisher,
            name: p.name,
            detail: detailRef(p.id),
          },
          publisher: publishers.find((x) => x.namespace === p.publisher) ?? null,
        }))

  const updates = () =>
    installed.flatMap((item) => {
      if (registry === "never-reached") return []
      const source = plugins.find((p) => p.plugin.id === item.id)
      if (!source) return []
      const offer = offeredVersion(source.plugin.versions, revocations(item.id), environment)
      if (!offer?.compatibility.compatible) return []
      if (compareSemver(offer.version.version, item.version) <= 0) return []
      return [
        {
          id: item.id,
          from: item.version,
          to: offer.version.version,
          publishedAt: offer.version.published_at,
          permissions: offer.version.permissions,
        },
      ]
    })

  const snapshot = (): MarketplaceSnapshot => {
    if (options.failList) {
      throw new PluginError(
        "failed",
        "The background agent did not answer.",
        "Check Settings › Background Agent, then try again.",
      )
    }
    return {
      environment,
      registry: {
        state: registry,
        checkedAt,
        verifiedAt: registry === "never-reached" ? undefined : NOW,
        message: REGISTRY_MESSAGES[registry],
      },
      catalog: catalog(),
      installed,
      updates: updates(),
    }
  }

  const requireFreshRegistry = () => {
    if (registry === "fresh") return
    if (registry === "verification-failed") {
      throw new PluginError(
        "verification-failed",
        "The registry did not verify, so nothing can be installed or updated.",
        "Try again later. Installed plugins keep running.",
      )
    }
    throw new PluginError(
      registry === "stale" ? "registry-stale" : "registry-unavailable",
      "The registry is not up to date, so nothing can be installed or updated.",
      "Connect to the internet and check for updates. Installed plugins keep running.",
    )
  }

  const versionOf = (id: string, value: string) => {
    const found = find(id).plugin.versions.find((v) => v.version === value)
    if (!found) throw new PluginError("not-found", `${id} has no version ${value}.`)
    if (found.yanked)
      throw new PluginError("yanked", `Version ${value} was withdrawn: ${found.yanked.reason}`)
    const revoked = revocationOf(revocations(id), value)
    if (revoked)
      throw new PluginError("revoked", `The registry revoked this version: ${revoked.reason}`)
    const compat = compatibility(found, environment)
    if (!compat.compatible) throw new PluginError("incompatible", compat.reasons.join(" "))
    return found
  }

  const update = (id: string, change: (item: InstalledPlugin) => InstalledPlugin | null) => {
    const item = installed.find((p) => p.id === id)
    if (!item) throw new PluginError("not-found", `${id} is not installed.`)
    installed = installed.flatMap((p) => {
      if (p.id !== id) return [p]
      const next = change(p)
      return next ? [next] : []
    })
  }

  return {
    source: "fixtures",
    get registryState() {
      return registry
    },
    simulate(next) {
      registry = next
    },
    listPlugins: () => reply(snapshot),
    getPluginDetail: (id) =>
      reply((): PluginDetail => {
        const { plugin: p } = find(id)
        const entry = catalog().find((c) => c.entry.id === id)
        if (!entry) throw new PluginError("not-found", `The registry has no plugin ${id}.`)
        return {
          plugin: p,
          entry: entry.entry,
          publisher: entry.publisher,
          revocations: revocations(id),
        }
      }),
    refreshRegistry: () =>
      reply(() => {
        checkedAt = new Date().toISOString()
        return snapshot()
      }),
    installPlugin: (request: InstallRequest) =>
      reply(() => {
        requireFreshRegistry()
        if (installed.some((p) => p.id === request.id)) {
          throw new PluginError("failed", `${request.id} is already installed.`)
        }
        const target = versionOf(request.id, request.version)
        const missing = grantsOf(target.permissions).filter((g) => !request.consent.includes(g.key))
        if (missing.length > 0) {
          throw new PluginError(
            "consent-required",
            "Every permission needs your consent to install.",
          )
        }
        if (BAD_PACKAGES.has(request.id)) {
          throw new PluginError(
            "verification-failed",
            "Verification failed: the downloaded package does not match the hash the registry signed. Nothing was installed.",
            "Try again later. If it keeps failing, report the plugin to the registry maintainers.",
          )
        }
        const p = find(request.id).plugin
        installed = [
          ...installed,
          {
            id: p.id,
            name: p.name,
            publisher: p.publisher,
            version: target.version,
            source: "registry",
            enabled: true,
            permissions: target.permissions,
            grants: stored(target.permissions, target.version),
            installedAt: new Date().toISOString(),
            problems: [],
          },
        ]
        return snapshot()
      }),
    approvePluginUpdate: (request) =>
      reply(() => {
        requireFreshRegistry()
        const target = versionOf(request.id, request.version)
        update(request.id, (item) => {
          const change = compareGrants(item.grants, target.permissions)
          if (change.added.some((g) => !request.consent.includes(g.key))) {
            throw new PluginError(
              "consent-required",
              "This update asks for new permissions; it needs your consent.",
            )
          }
          const grantedAt = new Date().toISOString()
          return {
            ...item,
            version: target.version,
            previousVersion: item.version,
            permissions: target.permissions,
            grants: [
              ...change.kept.map(({ grant, stored: s }) => ({
                ...s,
                permission: grant.permission,
              })),
              ...change.added.map((grant): StoredGrant => ({
                key: grant.key,
                permission: grant.permission,
                enabled: true,
                grantedAt,
                grantedForVersion: target.version,
                source: "update",
              })),
            ],
            problems: item.problems.filter((p) => p.kind !== "yanked" && p.kind !== "broken"),
          }
        })
        return snapshot()
      }),
    rollbackPlugin: (id) =>
      reply(() => {
        update(id, (item) => {
          if (!item.previousVersion) {
            throw new PluginError("failed", `${item.name} has no previous version to roll back to.`)
          }
          return {
            ...item,
            version: item.previousVersion,
            previousVersion: item.version,
            problems: item.problems.filter((p) => p.kind !== "broken"),
          }
        })
        return snapshot()
      }),
    uninstallPlugin: (id) =>
      reply(() => {
        update(id, () => null)
        return snapshot()
      }),
    setPluginEnabled: (id, enabled, consent = []) =>
      reply(() => {
        update(id, (item) => {
          if (enabled && item.problems.some((p) => p.kind === "revoked")) {
            throw new PluginError(
              "revoked",
              "The registry revoked this plugin; it cannot be enabled.",
              "Uninstall it, or install a version the registry has not revoked.",
            )
          }
          return {
            ...item,
            enabled,
            grants: item.grants.map((g) =>
              consent.includes(g.key) ? { ...g, enabled: true, source: "user" as const } : g,
            ),
          }
        })
        return snapshot()
      }),
    setPluginGrant: (id, key, enabled) =>
      reply(() => {
        update(id, (item) => ({
          ...item,
          grants: item.grants.map((g) =>
            g.key === key ? { ...g, enabled, source: "user" as const } : g,
          ),
        }))
        return snapshot()
      }),
  }
}

function detailRef(id: string) {
  return { path: `plugins/${id}/${HASH}2.json`, sha256: `${HASH}2`, length: 2048 }
}
