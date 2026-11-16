import { describe, expect, it } from "vitest"

import { HELLO, createFakePluginClient, readyRuntime } from "@/test/plugins"

import {
  availabilityOf,
  availabilityOfError,
  compareSemver,
  installBlock,
  loadSnapshot,
  offeredVersion,
  updateCandidates,
} from "./model"
import { PluginError } from "./api"
import { describeHost, grantOfInfo, grantsByTier, grantsOf } from "./permissions"
import type { AvailablePlugin, MarketplaceSnapshot } from "./types"

describe("runtime availability", () => {
  it("tells the runtime states apart", () => {
    const off = { ...readyRuntime(), available: false }
    expect(availabilityOf(readyRuntime()).kind).toBe("ready")
    expect(
      availabilityOf(readyRuntime({ executionSupported: false, platform: "windows" })).kind,
    ).toBe("platformUnsupported")
    expect(availabilityOf({ ...off, built: false, enabled: false }).kind).toBe("notBuilt")
    expect(availabilityOf({ ...off, enabled: false }).kind).toBe("notEnabled")
    expect(availabilityOf(off).kind).toBe("unavailable")
  })

  it("turns refusals of the runtime query into states", () => {
    expect(availabilityOfError(new PluginError("agentNotRunning", "no agent"))).toEqual({
      kind: "agentNotRunning",
      message: "no agent",
    })
    expect(availabilityOfError({ reason: "agentTooOld", message: "old" })).toEqual({
      kind: "agentTooOld",
      message: "old",
    })
    expect(availabilityOfError(new PluginError("communication", "broken"))).toBeNull()
  })
})

describe("versions", () => {
  it("orders SemVer, pre-releases first", () => {
    const sorted = ["1.0.0", "1.0.0-rc.1", "0.9.0", "1.0.0-alpha.2", "1.0.0-beta.1", "1.10.0"].sort(
      compareSemver,
    )
    expect(sorted).toEqual([
      "0.9.0",
      "1.0.0-alpha.2",
      "1.0.0-beta.1",
      "1.0.0-rc.1",
      "1.0.0",
      "1.10.0",
    ])
  })

  it("offers the latest installable version", () => {
    const plugin: AvailablePlugin = {
      id: "a.b",
      name: "B",
      summary: "",
      publisher: "a",
      latest: "2.0.0",
      versions: [
        { version: "2.0.0", permissions: [], incompatible: ["needs DeckPilot 9"] },
        { version: "1.1.0", permissions: [], withdrawn: "yanked" },
        { version: "1.0.0", permissions: [] },
      ],
    }
    expect(offeredVersion(plugin)?.version).toBe("1.0.0")
    expect(offeredVersion({ ...plugin, versions: plugin.versions.slice(0, 2) })).toBeNull()
  })
})

describe("snapshot", () => {
  it("reads installed plugins in full and keeps a registry failure apart", async () => {
    const client = createFakePluginClient({
      helloInstalled: true,
      registryError: { reason: "registryUnavailable", message: "signature mismatch" },
    })
    const snapshot = await loadSnapshot(client, readyRuntime())
    expect(snapshot.installed.map((p) => p.summary.id)).toEqual([HELLO])
    expect(snapshot.installed[0]?.permissions[0]?.status).toBe("granted")
    expect(snapshot.available).toEqual([])
    expect(snapshot.registryError).toEqual({
      reason: "registryUnavailable",
      message: "signature mismatch",
    })
  })

  it("finds updates: offered by the registry, or staged by the agent", async () => {
    const client = createFakePluginClient({ helloInstalled: true })
    const snapshot = await loadSnapshot(client, readyRuntime())
    expect(updateCandidates(snapshot)).toMatchObject([{ kind: "available", version: "1.1.0" }])
    await client.updatePlugin(HELLO, "1.1.0", [])
    const staged = await loadSnapshot(client, readyRuntime())
    expect(updateCandidates(staged)).toMatchObject([
      { kind: "staged", pending: { version: "1.1.0" } },
    ])
  })

  it("blocks installs unless the registry is verified and fresh", () => {
    const base: MarketplaceSnapshot = {
      runtime: readyRuntime(),
      registry: { source: "/r", freshness: "fresh" },
      available: [],
      installed: [],
    }
    expect(installBlock(base)).toBeNull()
    expect(installBlock({ ...base, registry: null })).toMatch(/No plugin registry/)
    expect(installBlock({ ...base, registry: { source: "/r", freshness: "stale" } })).toMatch(
      /out of date/,
    )
    expect(
      installBlock({ ...base, registry: { source: "/r", freshness: "neverVerified" } }),
    ).toMatch(/not been verified/)
  })
})

describe("permissions", () => {
  it("keys grants exactly as the agent does", () => {
    const grants = grantsOf([
      {
        kind: "network",
        hosts: ["api.github.com", "*.example.com"],
        privateNetwork: true,
        justification: "Calls the API.",
      },
      { kind: "credentials", refs: ["token"], justification: "Signs in." },
      { kind: "notifications", justification: "Done." },
      { kind: "notifications", justification: "Twice is merged." },
      { kind: "profile", read: ["profile", "project"], justification: "Names." },
      { kind: "variables", read: ["git.*", "build.id"], justification: "Paths." },
      { kind: "storage", quotaKb: 1024, justification: "Cache." },
      { kind: "openUrl", hosts: ["github.com"], justification: "Links." },
      { kind: "background", minIntervalSeconds: 60, justification: "Polls." },
      { kind: "device", state: true, icons: true, justification: "Keys." },
      { kind: "device", icons: false, justification: "Nothing." },
    ])
    expect(grants.map((g) => g.key)).toEqual([
      "network:api.github.com",
      "network:*.example.com",
      "network:private-network",
      "credentials:token",
      "notifications",
      "profile",
      "variables:git.*",
      "variables:build.id",
      "storage",
      "openUrl:github.com",
      "background",
      "device:state",
      "device:icons",
    ])
    const tiers = grantsByTier(grants).map((g) => [g.tier, g.grants.length])
    expect(tiers).toEqual([
      ["high", 2],
      ["medium", 7],
      ["low", 4],
    ])
    expect(describeHost("*.example.com")).toBe(
      "any subdomain of example.com (https://*.example.com)",
    )
  })

  it("says where routed notifications go, and keeps them low tier", () => {
    const [plain] = grantsOf([{ kind: "notifications", justification: "Done." }])
    const [routed] = grantsOf([
      {
        kind: "notifications",
        routing: { desktop: "desktop", statusBar: "statusBar", minimumSeverity: "least" },
        justification: "Shows what your workflows ask for.",
      },
    ])
    expect(plain?.detail).toBe("Prefixed with the plugin's name.")
    expect(routed?.key).toBe("notifications")
    expect(routed?.tier).toBe("low")
    expect(routed?.detail).toMatch(/desktop and in the status bar as you set it/)
  })

  it("describes the agent's grants with the agent's tier", () => {
    const grant = grantOfInfo({
      key: "network:api.open-meteo.com",
      kind: "network",
      tier: "high",
      status: "granted",
      scope: { type: "host", methods: ["GET"] },
      justification: "Weather.",
    })
    expect(grant).toMatchObject({
      key: "network:api.open-meteo.com",
      tier: "high",
      title: "Send data to and receive data from https://api.open-meteo.com",
      justification: "Weather.",
    })
    expect(grant.detail).toContain("Methods: GET.")
    expect(
      grantOfInfo({
        key: "storage",
        kind: "storage",
        tier: "low",
        status: "notGranted",
        scope: { type: "storage", quotaKb: 512 },
      }).title,
    ).toBe("Store up to 512 KB of its own data")
  })

  it("says that pages open only in actions the user starts", () => {
    const grants = grantsOf([
      { kind: "openUrl", hosts: ["github.com"], justification: "Opens pull requests." },
    ])
    const [grant] = grants
    if (grant === undefined) throw new Error("no grant")
    expect(grants).toHaveLength(1)
    expect(grant).toMatchObject({
      key: "openUrl:github.com",
      tier: "medium",
      title: "Open links to https://github.com in your browser",
    })
    // What the broker enforces (G8): https pages of this host, only in a
    // run the user started, never a scheduled, event or startup run.
    expect(grant.detail).toContain("Only https pages of this host")
    expect(grant.detail).toContain("only in an action you start (a key, a gesture or the app")
    expect(grant.detail).toContain("never from a schedule, an event or startup")
    expect(grant.detail).toContain("At most one every 2 seconds.")
  })
})
