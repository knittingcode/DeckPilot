// `UpdateState` (types.ts) against the backend's serialization. The fixture
// is written by the Rust test `updates::tests::update_states_match_the_frontend_fixture`
// from the real `UpdateState`, so a field added, renamed or retyped on
// either side fails here or there.
import { describe, expect, it } from "vitest"

import type {
  DistributionChannel,
  PackageFormat,
  ReleaseChannel,
  UpdateCapabilities,
  UpdateInfo,
  UpdatePreferences,
  UpdateProviderKind,
  UpdateState,
} from "@/lib/types"

import fixture from "../test/fixtures/update-states.json"

type Kind = "string" | "boolean" | "number" | "object" | `${string}|null`

// `satisfies Record<keyof T, …>` makes each list exactly the type's keys:
// a missing key and an extra key are both compile errors.
const STATE = {
  currentVersion: "string",
  configured: "boolean",
  target: "string",
  endpoint: "string",
  preferences: "object",
  phase: "string",
  available: "object|null",
  progress: "object|null",
  error: "string|null",
  distributionChannel: "string",
  updateProvider: "string",
  releaseChannel: "string",
  supportsAutomaticUpdates: "boolean",
  packageFormat: "string|null",
  capabilities: "object",
  managedBy: "string|null",
  updateInstruction: "string|null",
  notConfiguredReason: "string|null",
  developmentOverride: "boolean",
} satisfies Record<keyof UpdateState, Kind>

const PREFERENCES = {
  mode: "string",
  automaticChecks: "boolean",
  checkIntervalHours: "number",
  lastCheck: "number|null",
  channel: "string",
} satisfies Record<keyof UpdatePreferences, Kind>

const INFO = {
  version: "string",
  notes: "string|null",
  date: "string|null",
} satisfies Record<keyof UpdateInfo, Kind>

const PROGRESS = {
  downloaded: "number",
  total: "number|null",
} satisfies Record<keyof NonNullable<UpdateState["progress"]>, Kind>

const CAPABILITIES = {
  selfUpdate: "boolean",
  automaticUpdates: "boolean",
  channelChoice: "boolean",
  versionCheck: "boolean",
} satisfies Record<keyof UpdateCapabilities, Kind>

const CHANNELS = {
  github: true,
  homebrew: true,
  winget: true,
  apt: true,
  flathub: true,
  mac_app_store: true,
  microsoft_store: true,
} satisfies Record<DistributionChannel, true>

const PROVIDERS = {
  github_release: true,
  external_package_manager: true,
  store_managed: true,
  none: true,
} satisfies Record<UpdateProviderKind, true>

const RELEASE_CHANNELS = { stable: true, beta: true, alpha: true } satisfies Record<
  ReleaseChannel,
  true
>

const FORMATS = {
  app: true,
  msi: true,
  nsis: true,
  deb: true,
  rpm: true,
  appimage: true,
  flatpak: true,
  mas_pkg: true,
  msix: true,
} satisfies Record<PackageFormat, true>

const PHASES = {
  idle: true,
  checking: true,
  downloading: true,
  ready: true,
  installing: true,
} satisfies Record<UpdateState["phase"], true>

function kindOf(value: unknown): string {
  if (value === null) return "null"
  return typeof value
}

/** `value` has exactly the keys of `shape`, each of its kind. */
function expectShape(value: unknown, shape: Record<string, Kind>, where: string) {
  expect(kindOf(value), where).toBe("object")
  const record = value as Record<string, unknown>
  expect(Object.keys(record).sort(), where).toEqual(Object.keys(shape).sort())
  for (const [key, kind] of Object.entries(shape)) {
    expect(kind.split("|"), `${where}.${key}`).toContain(kindOf(record[key]))
  }
}

function expectOneOf(value: unknown, values: Record<string, true>, where: string) {
  expect(Object.keys(values), where).toContain(value)
}

const states = Object.entries(fixture) as [string, Record<string, unknown>][]

describe("UpdateState matches the backend's serialization", () => {
  it.each(states)("%s", (name, state) => {
    expectShape(state, STATE, name)
    expectShape(state.preferences, PREFERENCES, `${name}.preferences`)
    expectShape(state.capabilities, CAPABILITIES, `${name}.capabilities`)
    if (state.available !== null) expectShape(state.available, INFO, `${name}.available`)
    if (state.progress !== null) expectShape(state.progress, PROGRESS, `${name}.progress`)
    expectOneOf(state.phase, PHASES, `${name}.phase`)
    expectOneOf(state.distributionChannel, CHANNELS, `${name}.distributionChannel`)
    expectOneOf(state.updateProvider, PROVIDERS, `${name}.updateProvider`)
    expectOneOf(state.releaseChannel, RELEASE_CHANNELS, `${name}.releaseChannel`)
    if (state.packageFormat !== null) {
      expectOneOf(state.packageFormat, FORMATS, `${name}.packageFormat`)
    }
    if (state.managedBy !== null) expectOneOf(state.managedBy, CHANNELS, `${name}.managedBy`)
  })

  it("covers every channel and provider the type names", () => {
    const seen = (key: string) => new Set(states.map(([, state]) => state[key]))
    expect([...seen("distributionChannel")].sort()).toEqual(Object.keys(CHANNELS).sort())
    expect([...seen("updateProvider")].sort()).toEqual(Object.keys(PROVIDERS).sort())
  })
})
