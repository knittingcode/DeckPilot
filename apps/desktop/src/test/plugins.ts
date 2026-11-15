// An in-memory stand-in for the agent's plugin backend, for tests only.
// It answers like protocol 10 does (shapes of `crates/deckpilot-ipc/src/
// plugins.rs`), with the refusals' `reason`s, so screens can be tested
// without Tauri. Never imported by the app.

import { type Mock, vi } from "vitest"

import type { ActionType } from "@/lib/types"
import { PluginError, type PluginClient } from "@/plugins/api"
import { grantsOf } from "@/plugins/permissions"
import type {
  AvailablePlugin,
  GrantInfo,
  KnownPlugin,
  ManifestPermission,
  NetworkPermission,
  PendingPluginUpdate,
  PermissionChanges,
  PluginCredentialSlot,
  PluginDetails,
  PluginEvent,
  PluginHealth,
  PluginInstallSource,
  PluginRetention,
  PluginRuntimeStatus,
  PluginSettingField,
  PluginSettings,
  PluginSettingValue,
  PluginStorageUsage,
  PluginSummary,
  PluginUpdateResult,
} from "@/plugins/types"

export const HELLO = "acme.hello-weather"
export const LIGHTS = "knit.home-lights"

const network = (hosts: string[], extra: Partial<NetworkPermission> = {}): NetworkPermission => ({
  kind: "network",
  hosts,
  methods: ["GET"],
  justification: "Reads the weather.",
  ...extra,
})

const NOTIFY: ManifestPermission = { kind: "notifications", justification: "Says when it is done." }

/** A 1×1 PNG, as the agent sends an installed plugin's icon. */
export const ICON =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII="

export function availablePlugins(): AvailablePlugin[] {
  return [
    {
      id: HELLO,
      name: "Hello Weather",
      summary: "Greets someone and shows the temperature.",
      description: "Greets someone by name and shows the current temperature of a city.",
      publisher: "acme",
      publisherInfo: {
        namespace: "acme",
        displayName: "Acme Tools",
        login: "acme",
        verified: true,
      },
      categories: ["weather", "fun"],
      license: "Apache-2.0",
      homepage: "https://acme.example/plugins",
      sourceRepository: "acme/plugins",
      latest: "1.1.0",
      versions: [
        {
          version: "1.1.0",
          permissions: [network(["api.open-meteo.com"]), NOTIFY],
          deckpilot: ">=0.5.0 <1.0.0",
          publishedAt: 1_767_225_600,
        },
        {
          version: "1.0.0",
          permissions: [network(["api.open-meteo.com"])],
          deckpilot: ">=0.5.0 <1.0.0",
          publishedAt: 1_764_547_200,
        },
      ],
    },
    {
      id: LIGHTS,
      name: "Home Lights",
      summary: "Switches lights.",
      publisher: "knit",
      publisherInfo: { namespace: "knit", login: "knit", verified: false },
      categories: ["home"],
      latest: "0.3.0",
      versions: [
        {
          version: "0.3.0",
          deckpilot: ">=0.6.0",
          platforms: ["macos", "linux"],
          permissions: [
            network(["api.lifx.com"], { privateNetwork: true, justification: "Talks to bulbs." }),
            { kind: "credentials", refs: ["token"], justification: "Signs in." },
            { kind: "device", state: true, icons: true, justification: "Shows state." },
          ],
        },
      ],
    },
    {
      id: "acme.future",
      name: "Future Keys",
      summary: "Needs a newer DeckPilot.",
      publisher: "acme",
      latest: "2.0.0",
      versions: [
        { version: "2.0.0", permissions: [], incompatible: ["requires DeckPilot >=9.0.0"] },
      ],
    },
    {
      id: "acme.gone",
      name: "Gone Plugin",
      summary: "Was withdrawn.",
      publisher: "acme",
      latest: "1.0.0",
      versions: [{ version: "1.0.0", permissions: [], withdrawn: "revoked: it leaked data" }],
    },
  ]
}

/**
 * The definitions the agent's engine registers for a plugin's two actions
 * (`PluginActionInfo.definition`), shaped like the hello-weather sample.
 */
export function pluginDefinitions(id: string): { greet: ActionType; temperature: ActionType } {
  return {
    greet: {
      id: `${id}.greet`,
      name: "Greet",
      description: "Says hello to someone.",
      category: "Fun",
      parameters: [
        { key: "name", label: "Name", kind: { type: "text", multiline: false }, required: true },
        {
          key: "style",
          label: "Style",
          kind: { type: "choice", options: ["plain", "excited"] },
          required: false,
          default: { type: "choice", value: "plain" },
        },
      ],
      requiresProject: false,
      node: {
        version: 1,
        icon: null,
        effect: "readOnly",
        retrySafe: true,
        idempotent: true,
        cancellable: true,
        changesContext: false,
        timeoutMs: 5000,
        outputs: [{ key: "greeting", label: "Greeting", kind: "text", always: true }],
      },
    },
    temperature: {
      id: `${id}.temperature`,
      name: "Current temperature",
      description: "Shows the temperature of a city.",
      category: "Weather",
      parameters: [
        { key: "city", label: "City", kind: { type: "text", multiline: false }, required: true },
      ],
      requiresProject: false,
    },
  }
}

export function readyRuntime(extra: Partial<PluginRuntimeStatus> = {}): PluginRuntimeStatus {
  return {
    available: true,
    built: true,
    enabled: true,
    executionSupported: true,
    platform: "macos",
    registry: { source: "/tmp/registry", freshness: "fresh", seq: 3 },
    ...extra,
  }
}

export interface FakeOptions {
  runtime?: PluginRuntimeStatus
  /** `getPluginRuntime` rejects with this (e.g. `{ reason: "agentNotRunning" }`). */
  runtimeError?: { reason: string; message: string }
  /** `listAvailablePlugins` rejects with this. */
  registryError?: { reason: string; message: string }
  available?: AvailablePlugin[]
  /** Installs `acme.hello-weather` 1.0.0, enabled with its grant. */
  helloInstalled?: boolean
  /** The state `acme.hello-weather` is installed in (default `enabled`). */
  helloState?: PluginSummary["state"]
  /** Its `stateReason`. */
  helloStateReason?: string
  /** Its health as the agent reports it. */
  helloHealth?: PluginHealth
  /** Adds a required setting without a default (`city`) to its manifest. */
  helloRequiresCity?: boolean
  /** Settings values saved for `acme.hello-weather`. */
  helloSettings?: Record<string, PluginSettingValue>
  /** Credential slots of `acme.hello-weather` with a stored secret. */
  helloCredentials?: string[]
  /** What its storage holds (`PluginDetails.storage`); absent like an older agent's. */
  helloStorage?: PluginStorageUsage
  /** Plugins `plugin_known` has besides the installed ones (e.g. uninstalled ones). */
  known?: KnownPlugin[]
}

/** The settings fields of the test plugin, as the agent returns them. */
export function helloSettingFields(requireCity = false): PluginSettingField[] {
  return [
    ...(requireCity
      ? [
          {
            key: "city",
            label: "City",
            description: "Where to read the weather.",
            kind: { type: "text", maxLength: 40 },
            required: true,
            portability: "portable" as const,
          },
        ]
      : []),
    {
      key: "units",
      label: "Units",
      description: "Celsius or Fahrenheit.",
      kind: { type: "choice", options: ["celsius", "fahrenheit"] },
      required: false,
      default: "celsius",
      portability: "portable",
    },
    {
      key: "refreshMinutes",
      label: "Refresh every",
      kind: { type: "integer", min: 1, max: 60 },
      required: false,
      default: 10,
      portability: "portable",
    },
    {
      key: "showFeelsLike",
      label: "Show “feels like”",
      kind: { type: "boolean" },
      required: false,
      default: false,
      portability: "portable",
    },
  ]
}

/** The credential slots the test plugin declares (manifest form). */
const HELLO_SLOTS = [
  {
    key: "apiKey",
    label: "API key",
    description: "Your Open-Meteo API key.",
    hosts: ["api.open-meteo.com"],
    schemes: ["bearer"],
    setupUrl: "https://open-meteo.com/en/pricing",
  },
]

/**
 * The agent's settings validation (`merge_settings`): every value given
 * must fit its field; the message lists every problem in the core's words.
 */
function validateSettings(
  plugin: string,
  fields: PluginSettingField[],
  values: Record<string, PluginSettingValue>,
): void {
  const problems: string[] = []
  for (const [key, value] of Object.entries(values)) {
    const field = fields.find((f) => f.key === key)
    if (!field) {
      problems.push(`unknown parameter \`${key}\``)
      continue
    }
    const kind = field.kind
    switch (kind.type) {
      case "integer":
        if (typeof value !== "number" || !Number.isInteger(value)) {
          problems.push(`\`${key}\` expects a whole number`)
        } else if (
          (typeof kind.min === "number" && value < kind.min) ||
          (typeof kind.max === "number" && value > kind.max)
        ) {
          problems.push(`parameter \`${key}\`: ${String(value)} is out of range`)
        }
        break
      case "boolean":
        if (typeof value !== "boolean") problems.push(`\`${key}\` expects true or false`)
        break
      case "choice": {
        const options = (kind.options as string[] | undefined) ?? []
        if (typeof value !== "string") problems.push(`\`${key}\` expects text`)
        else if (!options.includes(value)) {
          problems.push(`parameter \`${key}\`: \`${value}\` is not one of ${options.join(", ")}`)
        }
        break
      }
      default:
        if (typeof value !== "string") problems.push(`\`${key}\` expects text`)
        else if (typeof kind.maxLength === "number" && Array.from(value).length > kind.maxLength) {
          problems.push(
            `parameter \`${key}\`: must be at most ${String(kind.maxLength)} characters`,
          )
        }
    }
  }
  if (problems.length > 0) {
    refuse(
      "invalidData",
      `the settings of ${plugin} do not match their declaration: ${problems.join("; ")}`,
    )
  }
}

interface Installed {
  id: string
  version: string
  previousVersion?: string
  state: PluginSummary["state"]
  stateReason?: string
  health?: PluginHealth
  origin: PluginSummary["origin"]
  /** Grant key → status. */
  grants: Map<string, GrantInfo["status"]>
  pending?: PendingPluginUpdate
}

/** Settings and credentials, kept apart from the record like plugins.db does. */
interface PluginData {
  settings: Record<string, PluginSettingValue>
  /** Slot → when it was stored. Never the secret: the fake does not keep it. */
  credentials: Map<string, number>
}

function refuse(reason: string, message: string): never {
  throw new PluginError(reason, message)
}

export type FakePluginClient = { [K in keyof PluginClient]: Mock<PluginClient[K]> } & {
  emit: (event: PluginEvent) => void
  installed: () => string[]
  /** What is stored for a plugin: settings, and which slots have a secret. */
  stored: (id: string) => { settings: Record<string, PluginSettingValue>; credentials: string[] }
}

/** Every method is a `vi.fn`, so tests can check what was sent. */
export function createFakePluginClient(options: FakeOptions = {}): FakePluginClient {
  const available = options.available ?? availablePlugins()
  const installed = new Map<string, Installed>()
  const listeners = new Set<(event: PluginEvent) => void>()
  const data = new Map<string, PluginData>()
  const known = new Map<string, KnownPlugin>((options.known ?? []).map((k) => [k.id, k]))
  const dataOf = (id: string): PluginData => {
    let d = data.get(id)
    if (!d) {
      d = { settings: {}, credentials: new Map() }
      data.set(id, d)
    }
    return d
  }
  if (options.helloSettings || options.helloCredentials) {
    const d = dataOf(HELLO)
    Object.assign(d.settings, options.helloSettings ?? {})
    for (const key of options.helloCredentials ?? []) d.credentials.set(key, 1_767_225_600)
  }
  if (options.helloInstalled) {
    installed.set(HELLO, {
      id: HELLO,
      version: "1.0.0",
      state: options.helloState ?? "enabled",
      ...(options.helloStateReason ? { stateReason: options.helloStateReason } : {}),
      ...(options.helloHealth ? { health: options.helloHealth } : {}),
      origin: { type: "registry", registry: "/tmp/registry" },
      grants: new Map([["network:api.open-meteo.com", "granted"]]),
    })
  }

  const version = (id: string, v: string) => {
    const found = available.find((p) => p.id === id)?.versions.find((x) => x.version === v)
    if (!found) refuse("unknownPlugin", `the registry offers no ${id} ${v}`)
    return found
  }
  const declared = (p: Installed) => grantsOf(version(p.id, p.version).permissions)
  const info = (p: Installed): GrantInfo[] =>
    declared(p).map((g) => ({
      key: g.key,
      kind: g.key.split(":")[0] ?? g.key,
      tier: g.tier,
      status: p.grants.get(g.key) ?? "notGranted",
      scope: null,
      ...(g.justification ? { justification: g.justification } : {}),
    }))
  const get = (id: string) => installed.get(id) ?? refuse("notInstalled", `${id} is not installed`)
  const fieldsOf = (id: string) => helloSettingFields(id === HELLO && options.helloRequiresCity)
  const remember = (p: Installed) => {
    const plugin = available.find((a) => a.id === p.id)
    const before = known.get(p.id)
    known.set(p.id, {
      id: p.id,
      name: plugin?.name ?? p.id,
      publisher: plugin?.publisher ?? "",
      namespace: `plugins.${p.id}`,
      lastVersion: p.version,
      installed: true,
      firstSeenAt: before?.firstSeenAt ?? 1_767_225_600,
      lastSeenAt: 1_767_225_600,
    })
  }
  const settingsOf = (id: string): PluginSettings => {
    get(id)
    const fields = fieldsOf(id)
    const values = { ...dataOf(id).settings }
    const effective: Record<string, PluginSettingValue> = {}
    const missing: string[] = []
    for (const f of fields) {
      const v = values[f.key] ?? f.default
      if (v !== undefined) effective[f.key] = v
      else if (f.required) missing.push(f.key)
    }
    return { plugin: id, fields, values, effective, ...(missing.length > 0 ? { missing } : {}) }
  }
  const slotsOf = (id: string): PluginCredentialSlot[] => {
    const p = get(id)
    const stored = dataOf(id).credentials
    return HELLO_SLOTS.map((slot) => {
      const at = stored.get(slot.key)
      return {
        ...slot,
        set: at !== undefined,
        ...(at !== undefined ? { updatedAt: at } : {}),
        granted: p.grants.get(`credentials:${slot.key}`) === "granted",
      }
    })
  }
  const slot = (id: string, key: string) => {
    if (!HELLO_SLOTS.some((s) => s.key === key)) {
      refuse("invalidData", `${id} declares no credential slot \`${key}\``)
    }
  }
  const summary = (p: Installed): PluginSummary => {
    const plugin = available.find((a) => a.id === p.id)
    const definitions = pluginDefinitions(p.id)
    const runnable = p.state === "enabled"
    const reason = runnable ? {} : { unavailableReason: `the plugin is ${p.state}` }
    return {
      id: p.id,
      name: plugin?.name ?? p.id,
      publisher: plugin?.publisher ?? "",
      version: p.version,
      state: p.state,
      ...(p.stateReason ? { stateReason: p.stateReason } : {}),
      ...(p.health ? { health: p.health } : {}),
      origin: p.origin,
      ...(p.previousVersion ? { previousVersion: p.previousVersion } : {}),
      ...(p.pending ? { stagedVersion: p.pending.version } : {}),
      actions: [
        {
          action: "greet",
          actionType: `${p.id}.greet`,
          name: "Greet",
          available: runnable,
          ...reason,
          definition: definitions.greet,
        },
        {
          action: "temperature",
          actionType: `${p.id}.temperature`,
          name: "Current temperature",
          available: runnable,
          ...reason,
          definition: definitions.temperature,
        },
      ],
    }
  }
  const details = (p: Installed): PluginDetails => ({
    summary: summary(p),
    description: "A test plugin.",
    license: "Apache-2.0",
    permissions: info(p),
    ...(p.pending ? { pendingUpdate: p.pending } : {}),
    manifest: {
      engines: { deckpilot: ">=0.5.0 <1.0.0", pluginApi: "^1.0" },
      contributes: {
        actions: [
          {
            id: "greet",
            parameters: [{ key: "name", label: "Name", kind: { type: "text" } }],
          },
        ],
        settings: {
          fields: fieldsOf(p.id).map((f) => ({
            key: f.key,
            label: f.label,
            ...(f.description ? { description: f.description } : {}),
            kind: f.kind,
            required: f.required,
            ...(f.default !== undefined
              ? { default: { type: f.kind.type, value: f.default } }
              : {}),
          })),
        },
        credentials: HELLO_SLOTS,
      },
    },
    audit: [],
    ...(p.id === HELLO ? { icon: ICON } : {}),
    ...(p.id === HELLO && options.helloStorage ? { storage: options.helloStorage } : {}),
  })
  const changesTo = (p: Installed, to: string): PermissionChanges => {
    const before = new Set(declared(p).map((g) => g.key))
    const next = grantsOf(version(p.id, to).permissions)
    const added = next
      .filter((g) => !before.has(g.key))
      .map((g) => ({
        key: g.key,
        kind: g.key.split(":")[0] ?? g.key,
        tier: g.tier,
        status: "notGranted" as const,
        scope: null,
      }))
    const keys = new Set(next.map((g) => g.key))
    return {
      added,
      broadened: [],
      removed: [...before].filter((k) => !keys.has(k)),
      escalation: added.length > 0,
    }
  }
  const apply = (p: Installed, to: string, consent: string[]) => {
    const changes = changesTo(p, to)
    for (const key of changes.removed) p.grants.delete(key)
    for (const key of consent) p.grants.set(key, "granted")
    p.previousVersion = p.version
    p.version = to
    delete p.pending
    return changes
  }
  const emit = (event: PluginEvent) => {
    for (const listener of listeners) listener(event)
  }

  for (const p of installed.values()) remember(p)

  const client: FakePluginClient = {
    emit,
    installed: () => [...installed.keys()],
    stored: (id) => {
      const d = data.get(id)
      return {
        settings: { ...(d?.settings ?? {}) },
        credentials: [...(d?.credentials.keys() ?? [])],
      }
    },
    getRuntime: vi.fn(() =>
      options.runtimeError
        ? Promise.reject(new PluginError(options.runtimeError.reason, options.runtimeError.message))
        : Promise.resolve(options.runtime ?? readyRuntime()),
    ),
    listPlugins: vi.fn(() => Promise.resolve([...installed.values()].map(summary))),
    getPlugin: vi.fn((id: string) => Promise.resolve().then(() => details(get(id)))),
    listAvailablePlugins: vi.fn(() => {
      const registry = (options.runtime ?? readyRuntime()).registry
      if (options.registryError) {
        return Promise.reject(
          new PluginError(options.registryError.reason, options.registryError.message),
        )
      }
      if (!registry) {
        return Promise.reject(
          new PluginError("registryUnavailable", "no plugin registry is configured"),
        )
      }
      return Promise.resolve({
        registry,
        plugins: available.map((p) => {
          const local = installed.get(p.id)
          return local ? { ...p, installed: local.version } : p
        }),
      })
    }),
    installPlugin: vi.fn((source: PluginInstallSource, consent: string[]) =>
      Promise.resolve().then(() => {
        let id: string
        let v: string
        let origin: PluginSummary["origin"]
        if (source.type === "file") {
          if (!source.path.endsWith("hello-weather-1.0.0.dpplugin")) {
            refuse("invalidPackage", "the package is not a version the registry pins")
          }
          ;[id, v, origin] = [HELLO, "1.0.0", { type: "sideload" }]
        } else {
          id = source.plugin
          v = source.version ?? available.find((p) => p.id === id)?.latest ?? ""
          origin = { type: "registry", registry: "/tmp/registry" }
        }
        if (installed.has(id)) refuse("alreadyInstalled", `${id} is already installed`)
        const entry = version(id, v)
        if (entry.withdrawn) refuse("revoked", entry.withdrawn)
        const p: Installed = {
          id,
          version: v,
          state: "disabled",
          origin,
          grants: new Map(consent.map((k) => [k, "granted" as const])),
        }
        installed.set(id, p)
        remember(p)
        return details(p)
      }),
    ),
    enablePlugin: vi.fn((id: string, consent: string[]) =>
      Promise.resolve().then(() => {
        const p = get(id)
        for (const key of consent) p.grants.set(key, "granted")
        const missing = declared(p).filter((g) => p.grants.get(g.key) !== "granted")
        if (missing.length > 0)
          refuse("consentRequired", "every declared permission must be granted")
        p.state = "enabled"
        return details(p)
      }),
    ),
    disablePlugin: vi.fn((id: string) =>
      Promise.resolve().then(() => {
        const p = get(id)
        p.state = "disabled"
        return details(p)
      }),
    ),
    grantPermissions: vi.fn((id: string, keys: string[]) =>
      Promise.resolve().then(() => {
        const p = get(id)
        for (const key of keys) p.grants.set(key, "granted")
        return details(p)
      }),
    ),
    revokePermission: vi.fn((id: string, key: string) =>
      Promise.resolve().then(() => {
        const p = get(id)
        p.grants.set(key, "revoked")
        return details(p)
      }),
    ),
    updatePlugin: vi.fn(
      (id: string, to: string | undefined, consent: string[]): Promise<PluginUpdateResult> =>
        Promise.resolve().then(() => {
          const p = get(id)
          const target = to ?? available.find((a) => a.id === id)?.latest ?? p.version
          const changes = changesTo(p, target)
          if (changes.escalation && !changes.added.every((g) => consent.includes(g.key))) {
            p.pending = { version: target, changes }
            return { plugin: id, outcome: { type: "awaitingConsent", version: target, changes } }
          }
          apply(p, target, consent)
          return { plugin: id, outcome: { type: "applied", version: target, changes } }
        }),
    ),
    approveUpdate: vi.fn((id: string, consent: string[]): Promise<PluginUpdateResult> =>
      Promise.resolve().then(() => {
        const p = get(id)
        const pending = p.pending ?? refuse("invalidState", "no update is staged")
        if (!pending.changes.added.every((g) => consent.includes(g.key))) {
          refuse("consentRequired", "consent to every new permission")
        }
        const changes = apply(p, pending.version, consent)
        return { plugin: id, outcome: { type: "applied", version: p.version, changes } }
      }),
    ),
    declineUpdate: vi.fn((id: string) =>
      Promise.resolve().then(() => {
        const p = get(id)
        delete p.pending
        return details(p)
      }),
    ),
    rollbackPlugin: vi.fn((id: string) =>
      Promise.resolve().then(() => {
        const p = get(id)
        const previous = p.previousVersion ?? refuse("invalidState", "no previous version")
        p.previousVersion = p.version
        p.version = previous
        return details(p)
      }),
    ),
    uninstallPlugin: vi.fn((id: string, keep?: PluginRetention) =>
      Promise.resolve().then(() => {
        get(id)
        installed.delete(id)
        // Storage always goes; settings and credentials unless kept.
        const d = data.get(id)
        if (d && !keep?.settings) d.settings = {}
        if (d && !keep?.credentials) d.credentials.clear()
        const k = known.get(id)
        if (k) known.set(id, { ...k, installed: false })
      }),
    ),
    listKnownPlugins: vi.fn(() => Promise.resolve([...known.values()])),
    getSettings: vi.fn((id: string) => Promise.resolve().then(() => settingsOf(id))),
    setSettings: vi.fn((id: string, values: Record<string, PluginSettingValue | null>) =>
      Promise.resolve().then(() => {
        get(id)
        const merged: Record<string, PluginSettingValue> = Object.fromEntries(
          Object.entries({ ...dataOf(id).settings, ...values }).filter(
            (entry): entry is [string, PluginSettingValue] => entry[1] !== null,
          ),
        )
        validateSettings(id, fieldsOf(id), merged)
        dataOf(id).settings = merged
        return settingsOf(id)
      }),
    ),
    listCredentials: vi.fn((id: string) => Promise.resolve().then(() => slotsOf(id))),
    setCredential: vi.fn((id: string, key: string, secret: string) =>
      Promise.resolve().then(() => {
        get(id)
        slot(id, key)
        if (secret.length === 0 || /[\r\n\0]/.test(secret)) {
          refuse("invalidData", "a secret must be between 1 and 16384 bytes")
        }
        // Only that a secret is stored; the fake forgets the secret itself.
        dataOf(id).credentials.set(key, 1_767_312_000)
        return slotsOf(id)
      }),
    ),
    deleteCredential: vi.fn((id: string, key: string) =>
      Promise.resolve().then(() => {
        get(id)
        slot(id, key)
        dataOf(id).credentials.delete(key)
        return slotsOf(id)
      }),
    ),
    invokeAction: vi.fn((id: string, action: string, params: Record<string, unknown>) =>
      Promise.resolve().then(() => {
        const p = get(id)
        if (p.state !== "enabled") refuse("notEnabled", `${id} is not enabled`)
        if (action === "greet") {
          const name = typeof params.name === "string" ? params.name : "you"
          return { status: "ok" as const, message: `Hello, ${name}.`, data: { greeting: name } }
        }
        if (p.grants.get("network:api.open-meteo.com") !== "granted") {
          return {
            status: "failed" as const,
            code: "not-permitted",
            message: "network:api.open-meteo.com is not granted",
            retryable: false,
          }
        }
        return { status: "ok" as const, data: { celsius: 21 } }
      }),
    ),
    pickPackage: vi.fn(() => Promise.resolve<string | null>(null)),
    subscribe: vi.fn((listener: (event: PluginEvent) => void) => {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    }),
  }
  return client
}
