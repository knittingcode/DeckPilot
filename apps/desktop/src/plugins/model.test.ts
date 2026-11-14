import { describe, expect, it } from "vitest"

import { PluginError } from "./api"
import {
  compareSemver,
  compatibility,
  offeredVersion,
  satisfiesApi,
  satisfiesRange,
} from "./compat"
import { DEFAULT_FIXTURE_ENVIRONMENT, createFixtureClient } from "./fixtures"
import { compareGrants, describeHost, grantsByTier, grantsOf } from "./permissions"
import type { PluginPermission, RegistryPluginVersion, StoredGrant } from "./types"

function version(value: string, extra: Partial<RegistryPluginVersion> = {}): RegistryPluginVersion {
  return {
    version: value,
    published_at: "2026-11-01T00:00:00Z",
    engines: { deckpilot: ">=0.6.0 <1.0.0", pluginApi: "^1.0" },
    permissions: [],
    manifest_sha256: "0".repeat(64),
    source: { repository: "fixture/x", tag: `v${value}`, commit: "0".repeat(40) },
    package: { url: "https://example.invalid", sha256: "0".repeat(64), size: 1 },
    builder: "1.0.0",
    ...extra,
  }
}

function stored(permissions: PluginPermission[], revoked: string[] = []): StoredGrant[] {
  return grantsOf(permissions).map((g) => ({
    key: g.key,
    permission: g.permission,
    enabled: !revoked.includes(g.key),
    grantedAt: "2026-11-01T00:00:00Z",
    grantedForVersion: "1.0.0",
    source: "install",
  }))
}

describe("plugin compatibility", () => {
  it("orders the release-tag grammar, pre-releases first", () => {
    const sorted = ["1.0.0", "1.0.0-rc.1", "0.9.0", "1.0.0-alpha.2", "1.0.0-beta.1"].sort(
      compareSemver,
    )
    expect(sorted).toEqual(["0.9.0", "1.0.0-alpha.2", "1.0.0-beta.1", "1.0.0-rc.1", "1.0.0"])
  })

  it("checks engine ranges and plugin API levels", () => {
    expect(satisfiesRange("0.8.0", ">=0.8.0 <1.0.0")).toBe(true)
    expect(satisfiesRange("1.0.0", ">=0.8.0 <1.0.0")).toBe(false)
    expect(satisfiesRange("0.6.0-alpha.2", ">=0.6.0")).toBe(false)
    expect(satisfiesRange("0.6.0", "=0.6.0")).toBe(true)
    expect(satisfiesRange("0.6.0", "nonsense")).toBe(false)
    expect(satisfiesApi("1.2", "^1.0")).toBe(true)
    expect(satisfiesApi("1.0", "^1.2")).toBe(false)
    expect(satisfiesApi("2.0", "^1.0")).toBe(false)
  })

  it("explains every reason a version does not run here", () => {
    const result = compatibility(
      version("1.0.0", {
        engines: { deckpilot: ">=0.9.0", pluginApi: "^1.2" },
        platforms: ["windows"],
      }),
      DEFAULT_FIXTURE_ENVIRONMENT,
    )
    expect(result.compatible).toBe(false)
    expect(result.reasons).toEqual([
      "Requires DeckPilot >=0.9.0; this is DeckPilot 0.6.0.",
      "Requires plugin API ^1.2; this DeckPilot provides 1.0.",
      "Not available for macOS; supports Windows.",
    ])
    expect(
      compatibility(version("1.0.0"), { ...DEFAULT_FIXTURE_ENVIRONMENT, pluginsSupported: false })
        .reasons,
    ).toEqual(["Plugins are not available on macOS."])
  })

  it("offers the newest stable version that is neither withdrawn nor revoked", () => {
    const versions = [
      version("1.0.0"),
      version("1.1.0", { yanked: { at: "2026-11-01T00:00:00Z", reason: "Broken" } }),
      version("1.2.0"),
      version("1.3.0-beta.1"),
    ]
    const env = DEFAULT_FIXTURE_ENVIRONMENT
    expect(offeredVersion(versions, [], env)?.version.version).toBe("1.2.0")
    const yank = { id: "a.b", action: "yank" as const, reason: "x", at: "2026-11-01T00:00:00Z" }
    expect(offeredVersion(versions, [{ ...yank, versions: ["1.2.0"] }], env)?.version.version).toBe(
      "1.0.0",
    )
    expect(offeredVersion(versions, [{ ...yank, versions: "*" }], env)).toBeNull()
  })
})

describe("plugin permissions", () => {
  it("names exact origins and wildcard patterns precisely", () => {
    expect(describeHost("api.github.com")).toBe("https://api.github.com")
    expect(describeHost("api.example.com:8443")).toBe("https://api.example.com:8443")
    expect(describeHost("*.statuspage.io")).toBe(
      "any subdomain of statuspage.io (https://*.statuspage.io)",
    )
  })

  it("normalizes one grant per host, ref and read entry, with risk tiers", () => {
    const grants = grantsOf([
      {
        kind: "network",
        hosts: ["a.example.com", "b.example.com"],
        methods: ["GET"],
        privateNetwork: true,
        justification: "Talks to the service.",
      },
      { kind: "credentials", refs: ["token"], justification: "Authenticates requests." },
      { kind: "variables", read: ["system.clock.*"], justification: "Reads the clock." },
      { kind: "notifications", justification: "Tells you things." },
    ])
    expect(grants.map((g) => [g.key, g.tier])).toEqual([
      ["network:a.example.com", "medium"],
      ["network:b.example.com", "medium"],
      ["network:private", "high"],
      ["credentials:token", "high"],
      ["variables:system.clock.*", "medium"],
      ["notifications", "low"],
    ])
    expect(grants[0]?.title).toBe("Send data to and receive data from https://a.example.com")
    expect(grants[0]?.detail).toContain("Methods: GET.")
    // Secrets are described by their slot, never by a value.
    expect(grants[3]?.detail).toContain("The plugin gets a reference, not the value")
    expect(grantsByTier(grants).map((g) => g.label)).toEqual([
      "High risk",
      "Medium risk",
      "Low risk",
    ])
  })

  it("treats new hosts, kinds and broader parameters as escalations", () => {
    const before: PluginPermission[] = [
      { kind: "network", hosts: ["a.example.com"], methods: ["GET"], justification: "Reads data." },
      { kind: "storage", quotaKb: 512, justification: "Keeps history." },
      { kind: "notifications", justification: "Tells you things." },
    ]
    const after: PluginPermission[] = [
      {
        kind: "network",
        hosts: ["a.example.com", "b.example.com"],
        methods: ["GET"],
        justification: "Reads more data.",
      },
      { kind: "storage", quotaKb: 1024, justification: "Keeps more history." },
      { kind: "background", minIntervalSeconds: 60, justification: "Refreshes the key." },
    ]
    const change = compareGrants(stored(before), after)
    expect(change.added.map((g) => g.key)).toEqual([
      "network:b.example.com",
      "storage",
      "background",
    ])
    // A changed justification alone is not an escalation.
    expect(change.kept.map((k) => k.grant.key)).toEqual(["network:a.example.com"])
    expect(change.removed.map((g) => g.key)).toEqual(["notifications"])
    expect(compareGrants(stored(before), before.slice(0, 1)).added).toEqual([])
  })
})

describe("fixture client", () => {
  it("refuses an install without consent to every permission", async () => {
    const client = createFixtureClient()
    const grants = grantsOf(
      (await client.getPluginDetail("knit.home-lights")).plugin.versions[0]?.permissions ?? [],
    )
    const partial = grants.slice(1).map((g) => g.key)
    await expect(
      client.installPlugin({ id: "knit.home-lights", version: "0.3.0", consent: partial }),
    ).rejects.toMatchObject({ code: "consent-required" })
    const next = await client.installPlugin({
      id: "knit.home-lights",
      version: "0.3.0",
      consent: grants.map((g) => g.key),
    })
    expect(next.installed.some((p) => p.id === "knit.home-lights")).toBe(true)
  })

  it("refuses an update with new permissions until they are consented to", async () => {
    const client = createFixtureClient()
    await expect(
      client.approvePluginUpdate({ id: "acme.github-prs", version: "1.3.0", consent: [] }),
    ).rejects.toBeInstanceOf(PluginError)
    const next = await client.approvePluginUpdate({
      id: "acme.github-prs",
      version: "1.3.0",
      consent: ["openUrl:github.com"],
    })
    expect(next.installed.find((p) => p.id === "acme.github-prs")?.version).toBe("1.3.0")
  })

  it("refuses installs while the registry is not fresh, and revoked or withdrawn versions", async () => {
    const client = createFixtureClient({ registry: "stale" })
    await expect(
      client.installPlugin({
        id: "beta.chat-bridge",
        version: "1.0.0",
        consent: ["network:hooks.example.org"],
      }),
    ).rejects.toMatchObject({ code: "registry-stale" })
    client.simulate("fresh")
    await expect(
      client.installPlugin({ id: "beta.chat-bridge", version: "1.1.0", consent: [] }),
    ).rejects.toMatchObject({ code: "yanked" })
    await expect(client.setPluginEnabled("shady.clipboard-sync", true)).rejects.toMatchObject({
      code: "revoked",
    })
  })
})
