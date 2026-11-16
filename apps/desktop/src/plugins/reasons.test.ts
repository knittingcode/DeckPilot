// The agent's refusal reasons in plain words, and the pure helpers the
// marketplace screens show registry metadata with.
import { describe, expect, it } from "vitest"

import { PluginError, pluginErrorText, reasonTitle } from "./api"
import {
  categoriesOf,
  categoryLabel,
  declaredConfiguration,
  inCategory,
  manifestCompatibility,
  matchesQuery,
  platformsText,
} from "./model"
import type { AvailablePlugin } from "./types"

// Every reason of `deckpilot_ipc::plugins::reason` and of the app's
// `commands::plugins::reason`.
const REASONS = [
  "agentNotRunning",
  "agentTooOld",
  "communication",
  "noDialog",
  "pluginsUnavailable",
  "platformUnsupported",
  "locked",
  "notInstalled",
  "unknownPlugin",
  "storeUnavailable",
  "alreadyInstalled",
  "consentRequired",
  "unknownPermission",
  "registryUnavailable",
  "revoked",
  "incompatible",
  "invalidPackage",
  "unsupportedRuntime",
  "invalidState",
  "notEnabled",
  "hostFailed",
  "broken",
  "busy",
  "invalidParameters",
  "answerTooLarge",
]

describe("reason titles", () => {
  it("words every reason of the agent and the app on its own", () => {
    const titles = REASONS.map(reasonTitle)
    expect(titles).not.toContain("The request failed")
    expect(new Set(titles).size).toBe(REASONS.length)
  })

  it("gives unknown reasons and plain failures a generic title", () => {
    expect(reasonTitle("somethingNew")).toBe("The request failed")
    expect(reasonTitle("failed")).toBe("The request failed")
  })

  it("keeps the agent's message next to the title and the hint", () => {
    expect(
      pluginErrorText(new PluginError("invalidPackage", "the package is 12 bytes, pinned 13")),
    ).toEqual({
      title: "The package was refused",
      message: "the package is 12 bytes, pinned 13",
      hint: "Only package files whose bytes the verified registry pins can be installed.",
      reason: "invalidPackage",
    })
    expect(pluginErrorText(new PluginError("busy", "8 waiting"))).toMatchObject({
      title: "The plugin is busy",
      hint: "Wait for its running actions to finish, then try again.",
    })
  })
})

const plugin = (extra: Partial<AvailablePlugin>): AvailablePlugin => ({
  id: "acme.x",
  name: "X",
  summary: "Does x.",
  publisher: "acme",
  versions: [],
  ...extra,
})

describe("registry metadata", () => {
  it("names platforms, every platform when none is listed", () => {
    expect(platformsText(undefined)).toBe("macOS, Windows and Linux")
    expect(platformsText([])).toBe("macOS, Windows and Linux")
    expect(platformsText(["linux"])).toBe("Linux only")
    expect(platformsText(["macos", "windows", "linux"])).toBe("macOS, Windows and Linux")
    expect(platformsText(["macos", "freebsd"])).toBe("macOS and freebsd")
  })

  it("collects and words categories", () => {
    const plugins = [
      plugin({ categories: ["weather", "developer-tools"] }),
      plugin({ categories: ["weather"] }),
      plugin({}),
    ]
    expect(categoriesOf(plugins)).toEqual(["developer-tools", "weather"])
    expect(categoryLabel("developer-tools")).toBe("Developer tools")
    expect(inCategory(plugins[2] as AvailablePlugin, null)).toBe(true)
    expect(inCategory(plugins[2] as AvailablePlugin, "weather")).toBe(false)
    expect(inCategory(plugins[1] as AvailablePlugin, "weather")).toBe(true)
  })

  it("searches the publisher's display name too", () => {
    const item = plugin({
      publisherInfo: {
        namespace: "acme",
        displayName: "Acme Tools",
        login: "acme",
        verified: true,
      },
    })
    expect(matchesQuery(item, "tools")).toBe(true)
    expect(matchesQuery(item, "acme x")).toBe(true)
    expect(matchesQuery(item, "other")).toBe(false)
  })

  it("reads what an installed manifest needs and declares, ignoring what it does not know", () => {
    expect(
      manifestCompatibility({ engines: { deckpilot: ">=0.5.0" }, platforms: ["macos", 3] }),
    ).toEqual({ deckpilot: ">=0.5.0", platforms: ["macos"] })
    expect(manifestCompatibility(null)).toEqual({})
    expect(
      declaredConfiguration({
        contributes: {
          settings: { fields: [{ key: "units", label: "Units" }, { label: "no key" }] },
          credentials: [{ key: "token", label: "Token", description: "From the vendor." }],
        },
      }),
    ).toEqual({
      settings: [{ key: "units", label: "Units" }],
      credentials: [{ key: "token", label: "Token", description: "From the vendor." }],
    })
    expect(declaredConfiguration({ contributes: {} })).toEqual({ settings: [], credentials: [] })
  })
})
