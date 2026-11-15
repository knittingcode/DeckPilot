import { afterEach, describe, expect, it, vi } from "vitest"

import { backendListeners, emitBackend, resetBackend } from "@/test/tauri"

import { PluginError } from "./api"
import { PLUGIN_EVENT, createTauriPluginClient } from "./client"
import type { PluginEvent } from "./types"

const invoke = vi.hoisted(() => vi.fn())
vi.mock("@tauri-apps/api/core", () => ({ invoke }))
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

afterEach(() => {
  invoke.mockReset()
  resetBackend()
})

const client = createTauriPluginClient()

describe("the Tauri plugin client", () => {
  // Each method, the command it invokes with its exact arguments, and the
  // answer it returns as it is.
  it.each([
    ["getRuntime", () => client.getRuntime(), "get_plugin_runtime", undefined],
    ["listPlugins", () => client.listPlugins(), "list_plugins", undefined],
    ["getPlugin", () => client.getPlugin("acme.x"), "get_plugin", { plugin: "acme.x" }],
    [
      "listAvailablePlugins",
      () => client.listAvailablePlugins(),
      "list_available_plugins",
      { refresh: false },
    ],
    [
      "listAvailablePlugins (refresh)",
      () => client.listAvailablePlugins(true),
      "list_available_plugins",
      { refresh: true },
    ],
    [
      "installPlugin (registry)",
      () =>
        client.installPlugin({ type: "registry", plugin: "acme.x", version: "1.0.0" }, [
          "network:api.example.com",
          "notifications",
        ]),
      "install_plugin",
      {
        source: { type: "registry", plugin: "acme.x", version: "1.0.0" },
        consent: ["network:api.example.com", "notifications"],
      },
    ],
    [
      "installPlugin (file)",
      () => client.installPlugin({ type: "file", path: "/tmp/x.dpplugin" }, []),
      "install_plugin",
      { source: { type: "file", path: "/tmp/x.dpplugin" }, consent: [] },
    ],
    [
      "enablePlugin",
      () => client.enablePlugin("acme.x", ["notifications"]),
      "enable_plugin",
      { plugin: "acme.x", consent: ["notifications"] },
    ],
    ["disablePlugin", () => client.disablePlugin("acme.x"), "disable_plugin", { plugin: "acme.x" }],
    [
      "grantPermissions",
      () => client.grantPermissions("acme.x", ["storage"]),
      "grant_plugin_permissions",
      { plugin: "acme.x", permissions: ["storage"] },
    ],
    [
      "revokePermission",
      () => client.revokePermission("acme.x", "storage"),
      "revoke_plugin_permission",
      { plugin: "acme.x", permission: "storage" },
    ],
    [
      "updatePlugin",
      () => client.updatePlugin("acme.x", "1.1.0", []),
      "update_plugin",
      { plugin: "acme.x", version: "1.1.0", consent: [] },
    ],
    [
      "updatePlugin (latest)",
      () => client.updatePlugin("acme.x", undefined, ["notifications"]),
      "update_plugin",
      { plugin: "acme.x", version: null, consent: ["notifications"] },
    ],
    [
      "approveUpdate",
      () => client.approveUpdate("acme.x", ["network:b.example.com"]),
      "approve_plugin_update",
      { plugin: "acme.x", consent: ["network:b.example.com"] },
    ],
    [
      "declineUpdate",
      () => client.declineUpdate("acme.x"),
      "decline_plugin_update",
      { plugin: "acme.x" },
    ],
    [
      "rollbackPlugin",
      () => client.rollbackPlugin("acme.x"),
      "rollback_plugin",
      { plugin: "acme.x" },
    ],
    [
      "invokeAction",
      () => client.invokeAction("acme.x", "greet", { name: "Ada", loud: true, n: 2 }),
      "invoke_plugin_action",
      { plugin: "acme.x", action: "greet", params: { name: "Ada", loud: true, n: 2 } },
    ],
    ["pickPackage", () => client.pickPackage(), "pick_plugin_package", undefined],
  ] as const)("%s", async (_, request, command, args) => {
    const answer = { marker: command }
    invoke.mockResolvedValueOnce(answer)
    await expect(request()).resolves.toBe(answer)
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenCalledWith(command, args)
  })

  it("resolves uninstall without a value", async () => {
    invoke.mockResolvedValueOnce(null)
    await expect(client.uninstallPlugin("acme.x")).resolves.toBeUndefined()
    expect(invoke).toHaveBeenCalledWith("uninstall_plugin", { plugin: "acme.x" })
  })

  it("sends a copy of the consent, not the caller's array", async () => {
    const consent = ["notifications"]
    invoke.mockResolvedValueOnce({})
    await client.enablePlugin("acme.x", consent)
    const sent = (invoke.mock.calls[0]?.[1] as { consent: string[] }).consent
    expect(sent).toEqual(consent)
    expect(sent).not.toBe(consent)
  })
})

describe("plugin errors", () => {
  it("keeps the backend's reason and message, with a hint", async () => {
    invoke.mockRejectedValueOnce({
      reason: "consentRequired",
      message: "network:api.example.com is not granted",
    })
    const error = await client.enablePlugin("acme.x", []).catch((e: unknown) => e)
    expect(error).toBeInstanceOf(PluginError)
    expect(error).toMatchObject({
      reason: "consentRequired",
      message: "network:api.example.com is not granted",
      hint: "Consent to every permission the plugin declares.",
    })
  })

  it.each([
    ["agentNotRunning", "Start the agent in Settings › Background Agent, then try again."],
    [
      "agentTooOld",
      "Restart the agent in Settings › Background Agent so that the agent of this release runs.",
    ],
    [
      "pluginsUnavailable",
      "Plugins need an agent built with the `plugins` feature and started with --enable-plugins.",
    ],
    [
      "platformUnsupported",
      "Plugins can be installed and inspected here, but only run where the plugin sandbox is verified: macOS, Linux and Windows.",
    ],
    [
      "invalidPackage",
      "Only package files whose bytes the verified registry pins can be installed.",
    ],
  ])("maps %s", async (reason, hint) => {
    invoke.mockRejectedValueOnce({ reason, message: "refused" })
    await expect(client.listPlugins()).rejects.toMatchObject({ reason, message: "refused", hint })
  })

  it("keeps reasons it does not know, without a hint", async () => {
    invoke.mockRejectedValueOnce({ reason: "somethingNew", message: "new refusal" })
    const error = (await client.listPlugins().catch((e: unknown) => e)) as PluginError
    expect(error.reason).toBe("somethingNew")
    expect(error.hint).toBeUndefined()
  })

  it("reports plain string errors as failures", async () => {
    invoke.mockRejectedValueOnce("the configuration is locked")
    await expect(client.listPlugins()).rejects.toMatchObject({
      reason: "failed",
      message: "the configuration is locked",
    })
  })
})

describe("plugin events", () => {
  it("forwards plugin events until unsubscribed and asks the app to watch", async () => {
    invoke.mockResolvedValue(null)
    const received: PluginEvent[] = []
    const stop = client.subscribe((event) => received.push(event))
    expect(invoke).toHaveBeenCalledWith("watch_plugin_events")
    await vi.waitFor(() => {
      expect(backendListeners.get(PLUGIN_EVENT)?.size).toBe(1)
    })
    const progress: PluginEvent = {
      type: "progress",
      plugin: "acme.x",
      operation: "install",
      stage: "verifying",
    }
    const changed: PluginEvent = { type: "changed", plugin: "acme.x", state: "disabled" }
    emitBackend(PLUGIN_EVENT, progress)
    emitBackend(PLUGIN_EVENT, changed)
    expect(received).toEqual([progress, changed])
    stop()
    expect(backendListeners.get(PLUGIN_EVENT)?.size).toBe(0)
    emitBackend(PLUGIN_EVENT, changed)
    expect(received).toHaveLength(2)
  })

  it("drops a listener that was unsubscribed before it was registered", async () => {
    invoke.mockResolvedValue(null)
    const listener = vi.fn()
    const stop = client.subscribe(listener)
    stop()
    await Promise.resolve()
    await Promise.resolve()
    expect(backendListeners.get(PLUGIN_EVENT)?.size ?? 0).toBe(0)
    emitBackend(PLUGIN_EVENT, { type: "changed", plugin: "acme.x" })
    expect(listener).not.toHaveBeenCalled()
  })

  it("does not fail when the app cannot watch", async () => {
    invoke.mockRejectedValue("no agent")
    const stop = client.subscribe(vi.fn())
    await Promise.resolve()
    stop()
  })
})
