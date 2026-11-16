// @vitest-environment jsdom
// Servers the user enters (G2): the consent asks for exactly the server
// the agent normalized, never assumes a private network, and revokes.
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { installDomStubs } from "@/test/dom"
import { resetToasts, toast } from "@/test/toast"

import { PluginError } from "./api"
import { grantOfInfo, grantsOf } from "./permissions"
import type { PluginSettings, PluginUserOrigin } from "./types"
import { UserOriginsSection } from "./user-origins"
import { consentText } from "./user-origins-text"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeAll(installDomStubs)
beforeEach(resetToasts)
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

const PLUGIN = "Forge"

function origin(extra: Partial<PluginUserOrigin> = {}): PluginUserOrigin {
  return {
    setting: "apiUrl",
    label: "API URL",
    permission: "network:user-origin:apiUrl",
    status: "needsConsent",
    origin: "https://ghe.example.com/api/v3",
    privateNetworkAllowed: true,
    pathPrefixAllowed: true,
    methods: ["GET", "POST"],
    credentials: ["token"],
    ...extra,
  }
}

function settingsWith(o: PluginUserOrigin): PluginSettings {
  return { plugin: "acme.forge", fields: [], values: {}, effective: {}, origins: [o] }
}

function setup(o: PluginUserOrigin) {
  const grant = vi.fn((_setting: string, _origin: string, privateNetwork: boolean) =>
    Promise.resolve(
      settingsWith({
        ...o,
        status: "granted",
        privateNetworkGranted: privateNetwork,
        grantedAt: 1_767_225_600,
      }),
    ),
  )
  const revoke = vi.fn(() => Promise.resolve(settingsWith({ ...o, status: "needsConsent" })))
  const onSettings = vi.fn()
  const view = render(
    <UserOriginsSection
      plugin={PLUGIN}
      origins={[o]}
      grant={grant}
      revoke={revoke}
      onSettings={onSettings}
    />,
  )
  return { grant, revoke, onSettings, view, user: userEvent.setup() }
}

const row = () => within(screen.getByRole("list", { name: `Servers of ${PLUGIN}` }))

describe("the consent to a server the user entered", () => {
  it("names exactly the server, the methods and the secrets that go there", async () => {
    const { grant, onSettings, user } = setup(origin())
    const consent = row().getByRole("group", { name: "Allow https://ghe.example.com/api/v3?" })
    expect(consent).toHaveTextContent(
      "Allow Forge to send data and your “token” secret to https://ghe.example.com/api/v3? Requests (GET, POST) go to exactly this server and path, nowhere else.",
    )
    expect(row().getByText("Needs your consent")).toBeInTheDocument()
    // The private network is a separate choice, never pre-selected.
    const lan = within(consent).getByRole("checkbox", {
      name: "Also allow it on my private network",
    })
    expect(lan).not.toBeChecked()
    await user.click(within(consent).getByRole("button", { name: "Allow" }))
    await waitFor(() => {
      expect(onSettings).toHaveBeenCalled()
    })
    expect(grant).toHaveBeenCalledWith("apiUrl", "https://ghe.example.com/api/v3", false)
    expect(toast.success).toHaveBeenCalledWith("API URL: https://ghe.example.com/api/v3 allowed")
  })

  it("asks for the private network on its own and requires it for a local server", async () => {
    const { grant, user } = setup(
      origin({ origin: "https://ghe.internal", privateHost: true, credentials: [] }),
    )
    const consent = row().getByRole("group", { name: "Allow https://ghe.internal?" })
    expect(consent).toHaveTextContent("Allow Forge to send data to https://ghe.internal?")
    const allow = within(consent).getByRole("button", { name: "Allow" })
    expect(allow).toBeDisabled()
    const lan = within(consent).getByRole("checkbox", {
      name: "Also allow it on my private network",
    })
    expect(lan).toHaveAccessibleDescription(/Required: this is a local name or address\./)
    expect(lan).toHaveAccessibleDescription(/This computer and link-local addresses are never/)
    await user.click(lan)
    expect(allow).toBeEnabled()
    await user.click(allow)
    await waitFor(() => {
      expect(grant).toHaveBeenCalledWith("apiUrl", "https://ghe.internal", true)
    })
  })

  it("cannot allow a private server the setting does not allow", () => {
    setup(
      origin({
        origin: "https://10.0.0.5",
        privateHost: true,
        privateNetworkAllowed: false,
      }),
    )
    const consent = row().getByRole("group", { name: "Allow https://10.0.0.5?" })
    expect(within(consent).queryByRole("checkbox")).not.toBeInTheDocument()
    expect(within(consent).getByRole("button", { name: "Allow" })).toBeDisabled()
    expect(consent).toHaveTextContent("which this plugin's setting does not allow")
  })

  it("forgets the private-network choice when the server changes", async () => {
    const { view, user } = setup(origin())
    const lan = () => row().getByRole("checkbox", { name: "Also allow it on my private network" })
    await user.click(lan())
    expect(lan()).toBeChecked()
    view.rerender(
      <UserOriginsSection
        plugin={PLUGIN}
        origins={[origin({ origin: "https://other.example.com" })]}
        grant={vi.fn()}
        revoke={vi.fn()}
        onSettings={vi.fn()}
      />,
    )
    expect(lan()).not.toBeChecked()
  })

  it("shows the agent's refusal", async () => {
    const { grant, user } = setup(origin())
    grant.mockRejectedValueOnce(
      new PluginError("invalidData", "the setting `apiUrl` does not hold `x`; save it first"),
    )
    await user.click(row().getByRole("button", { name: "Allow" }))
    expect(await row().findByRole("alert")).toHaveTextContent(/save it first/)
  })

  it("shows a granted server and revokes it", async () => {
    const { revoke, onSettings, user } = setup(
      origin({ status: "granted", privateNetworkGranted: true, grantedAt: 1_767_225_600 }),
    )
    expect(row().queryByRole("group")).not.toBeInTheDocument()
    expect(row().getByText("Allowed")).toBeInTheDocument()
    expect(row().getByText("Private network")).toBeInTheDocument()
    expect(row().getByText(/“token” is sent there\./)).toBeInTheDocument()
    await user.click(row().getByRole("button", { name: "Revoke API URL" }))
    await waitFor(() => {
      expect(onSettings).toHaveBeenCalled()
    })
    expect(revoke).toHaveBeenCalledWith("apiUrl")
  })

  it("explains every other state without offering a consent", () => {
    for (const [status, text] of [
      ["unset", /No server is set/],
      ["declared", /allowed when you installed it/],
      ["slotNotGranted", /“network:user-origin:apiUrl” first/],
      ["invalid", /The setting must start with https:\/\//],
    ] as const) {
      const { view } = setup(
        origin({
          status,
          ...(status === "unset" || status === "invalid" ? { origin: undefined } : {}),
          ...(status === "invalid" ? { problem: "must start with https://" } : {}),
        }),
      )
      expect(row().getByText(text)).toBeInTheDocument()
      expect(row().queryByRole("button", { name: "Allow" })).not.toBeInTheDocument()
      view.unmount()
    }
  })
})

describe("the install consent for an origin setting", () => {
  it("is keyed as the agent keys it, with its tier", () => {
    const grants = grantsOf([
      {
        kind: "network",
        hosts: ["api.github.com"],
        methods: ["GET"],
        userOrigins: [{ setting: "apiUrl", pathPrefix: true, privateNetwork: true }],
        justification: "Reads pull requests.",
      },
    ])
    expect(grants.map((g) => g.key)).toEqual([
      "network:api.github.com",
      "network:user-origin:apiUrl",
    ])
    const slot = grants[1]
    expect(slot?.tier).toBe("high")
    expect(slot?.title).toBe("Connect to a server you enter in its setting “apiUrl”")
    expect(slot?.detail).toMatch(/asks again whenever the setting changes/)
    expect(slot?.detail).toMatch(/only if you allow that for it separately/)
    const plain = grantOfInfo({
      key: "network:user-origin:apiUrl",
      kind: "network",
      tier: "medium",
      status: "granted",
      scope: { type: "userOriginSlot", methods: ["GET"] },
    })
    expect(plain.tier).toBe("medium")
    expect(plain.detail).toMatch(/Methods: GET\./)
    expect(plain.detail).not.toMatch(/local network/)
  })

  it("describes the consent text without a path or secrets", () => {
    expect(
      consentText("Forge", origin({ origin: "https://ghe.example.com", credentials: [] })),
    ).toBe(
      "Allow Forge to send data to https://ghe.example.com? Requests (GET, POST) go to exactly this server, nowhere else.",
    )
  })
})
