// @vitest-environment jsdom
// Module migrations in the app (experimental, VITE_DECKPILOT_PLUGINS=1):
// the notice in Settings › Plugins, the consent dialog, Retry, the
// `bundled` origin, and the legacy action types the editors keep
// (docs/marketplace/module-migration.md §5.2, §5.5-5.7). The agent is the
// in-memory stand-in of test/plugins.ts.
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { creatableTypes } from "@/lib/modules"
import {
  pluginActionUnavailable,
  pluginModules,
  typeAvailability,
  withPluginModules,
  withPluginTypes,
} from "@/lib/plugin-actions"
import type { ActionType, ModuleInfo } from "@/lib/types"
import { ActionsPage } from "@/pages/actions"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { action, builtInModules, moduleInfo, profileView } from "@/test/fixtures"
import {
  GITHUB_SECRET,
  createFakePluginClient,
  githubMigration,
  type FakeOptions,
} from "@/test/plugins"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

import { PluginClientContext } from "./api"
import { PluginsSettings } from "./marketplace"
import { MigrationConsentDialog } from "./migration-dialog"
import { isBundledPlugin } from "./model"
import type { ModuleMigration, PluginDetails, PluginSummary } from "./types"
import { usePluginModules } from "./use-plugin-modules"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
// The default client must never be created in these tests.
vi.mock("./client", () => ({
  createTauriPluginClient: () => {
    throw new Error("the Tauri client is not used in tests")
  },
}))

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
})
afterEach(() => {
  cleanup()
  vi.unstubAllEnvs()
})

function renderMarketplace(options: FakeOptions) {
  const client = createFakePluginClient(options)
  const result = renderWithApp(
    <PluginClientContext value={client}>
      <PluginsSettings />
    </PluginClientContext>,
  )
  return { ...result, client }
}

const notice = () => screen.findByRole("list", { name: "Module migrations" })
const entry = async (name: string) =>
  within(
    within(await notice())
      .getByText(name, { selector: "span" })
      .closest("li") as HTMLElement,
  )

describe("the migration notice", () => {
  it("says per module whether it moves, waits for consent or stays built-in, with the reason", async () => {
    renderMarketplace({
      migrations: [
        githubMigration(),
        {
          ...githubMigration({
            phase: "failed",
            attempts: 3,
            reason: "health check: required settings are not set: org",
          }),
          module: "gitlab",
          moduleName: "GitLab",
          plugin: "deckpilot.gitlab",
          pluginName: "GitLab",
        },
        {
          ...githubMigration({
            phase: "notEligible",
            reason: "`siteUrl` is not the default origin (user-entered origins need G2)",
          }),
          module: "jira",
          moduleName: "Jira",
          plugin: "deckpilot.jira",
          pluginName: "Jira",
        },
        {
          ...githubMigration({ phase: "installed" }),
          module: "notifications",
          moduleName: "Notifications",
          plugin: "deckpilot.notifications",
          pluginName: "Notifications",
        },
      ],
    })
    const github = await entry("GitHub")
    expect(github.getByText("Waiting for your consent")).toBeInTheDocument()
    expect(github.getByText(/waiting for your consent: credentials:token/)).toBeInTheDocument()
    expect(github.getByRole("button", { name: /Review/ })).toBeEnabled()
    expect(github.queryByRole("button", { name: /Retry/ })).not.toBeInTheDocument()

    const gitlab = await entry("GitLab")
    expect(gitlab.getByText("Staying built-in")).toBeInTheDocument()
    expect(gitlab.getByText(/failed 3 times/)).toBeInTheDocument()
    expect(gitlab.getByText("3 of 3 automatic attempts")).toBeInTheDocument()
    expect(gitlab.getByText(/required settings are not set: org/)).toBeInTheDocument()
    expect(gitlab.getByRole("button", { name: "Retry: GitLab" })).toBeEnabled()

    const jira = await entry("Jira")
    expect(jira.getByText("Staying built-in")).toBeInTheDocument()
    expect(jira.getByText(/user-entered origins need G2/)).toBeInTheDocument()
    expect(jira.queryByRole("button")).not.toBeInTheDocument()

    expect((await entry("Notifications")).getByText("Moving to a plugin")).toBeInTheDocument()
  })

  it("retries a failed migration and shows where it stands then", async () => {
    const { user, client } = renderMarketplace({
      migrations: [githubMigration({ phase: "failed", attempts: 3, reason: "health check: x" })],
    })
    const github = await entry("GitHub")
    await user.click(github.getByRole("button", { name: "Retry: GitHub" }))
    expect(client.retryModuleMigration).toHaveBeenCalledWith("github")
    await waitFor(async () => {
      expect((await entry("GitHub")).getByText("Moved to a plugin")).toBeInTheDocument()
    })
    expect(toast.success).toHaveBeenCalledWith("GitHub: trying again")
  })

  it("follows the agent's migration events", async () => {
    const { client } = renderMarketplace({ migrations: [githubMigration()] })
    await entry("GitHub")
    const calls = client.listModuleMigrations.mock.calls.length
    act(() => {
      client.emit({
        type: "migrationChanged",
        module: "github",
        plugin: "deckpilot.github",
        phase: "cutOver",
      })
    })
    await waitFor(() => {
      expect(client.listModuleMigrations.mock.calls.length).toBeGreaterThan(calls)
    })
  })

  it("shows nothing when no module moves, and survives an agent without migrations", async () => {
    const { client } = renderMarketplace({})
    client.listModuleMigrations.mockRejectedValue(new Error("unknown variant"))
    await screen.findByRole("list", { name: "Plugins" })
    expect(screen.queryByRole("list", { name: "Module migrations" })).not.toBeInTheDocument()
  })

  it("explains actions that are unavailable because the user withdrew the plugin", async () => {
    renderMarketplace({
      migrations: [
        githubMigration({
          phase: "cutOver",
          route: "pluginWithdrawn",
          routeReason: "GitHub (deckpilot.github) was disabled by you",
        }),
      ],
    })
    const github = await entry("GitHub")
    expect(github.getByText("Actions unavailable")).toBeInTheDocument()
    expect(github.getByText(/was disabled by you/)).toBeInTheDocument()
  })
})

describe("the consent dialog", () => {
  it("lists exactly what is imported, by name, and approves item by item", async () => {
    const { user, client } = renderMarketplace({ migrations: [githubMigration()] })
    await user.click((await entry("GitHub")).getByRole("button", { name: /Review/ }))
    const dialog = within(
      await screen.findByRole("dialog", { name: "GitHub actions now run as a plugin" }),
    )
    const item = within(dialog.getByRole("listitem"))
    expect(item.getByText("Use your stored credential github.token")).toBeInTheDocument()
    expect(item.getByText("High risk")).toBeInTheDocument()
    const facts = within(item.getByLabelText("What is imported"))
    expect(facts.getByText("github.token")).toBeInTheDocument()
    expect(facts.getByText("credential")).toBeInTheDocument()
    expect(facts.getByText("token")).toBeInTheDocument()
    expect(facts.getByText("api.github.com")).toBeInTheDocument()
    // Nothing asks for or shows a value.
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
    expect(document.querySelector("input[type=password]")).toBeNull()
    expect(document.body).not.toHaveTextContent(GITHUB_SECRET)

    const confirm = dialog.getByRole("button", { name: "Confirm" })
    expect(confirm).toBeDisabled()
    expect(dialog.queryByRole("button", { name: /Allow all/ })).not.toBeInTheDocument()
    await user.click(item.getByRole("radio", { name: "Allow" }))
    expect(confirm).toBeEnabled()
    await user.click(confirm)
    expect(client.decideModuleMigration).toHaveBeenCalledWith("github", ["credentials:token"], [])
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect((await entry("GitHub")).getByText("Moved to a plugin")).toBeInTheDocument()
  })

  it("keeps the built-in when an item is declined, and asks again on request", async () => {
    const { user, client } = renderMarketplace({
      migrations: [githubMigration({ privateNetwork: true })],
    })
    await user.click((await entry("GitHub")).getByRole("button", { name: /Review/ }))
    const dialog = within(await screen.findByRole("dialog"))
    const [token, network] = dialog.getAllByRole("listitem")
    if (!token || !network) throw new Error("two items")
    expect(within(network).getByText(/Private network access is never granted/)).toBeVisible()
    await user.click(within(token).getByRole("radio", { name: "Allow" }))
    // Every item needs its own decision.
    expect(dialog.getByRole("button", { name: "Confirm" })).toBeDisabled()
    expect(dialog.getByText("1 of 2 items still need your decision.")).toBeInTheDocument()
    await user.click(within(network).getByRole("radio", { name: "Don't allow" }))
    expect(dialog.getByText(/GitHub stays built-in; you can ask again later/)).toBeInTheDocument()
    await user.click(dialog.getByRole("button", { name: "Confirm" }))
    expect(client.decideModuleMigration).toHaveBeenCalledWith(
      "github",
      ["credentials:token"],
      ["network:private-network"],
    )
    const github = await entry("GitHub")
    await waitFor(() => {
      expect(github.getByText("Staying built-in")).toBeInTheDocument()
    })
    expect(github.getByText(/you declined network:private-network/)).toBeInTheDocument()
    await user.click(github.getByRole("button", { name: "Ask again: GitHub" }))
    expect(client.retryModuleMigration).toHaveBeenCalledWith("github")
    await waitFor(async () => {
      expect((await entry("GitHub")).getByText("Waiting for your consent")).toBeInTheDocument()
    })
  })

  it("asks for the server the built-in used, and separately for its private network", async () => {
    const server = "https://gitlab.internal/gitlab/api/v4"
    const base = githubMigration()
    const token = base.consent?.[0]
    if (!token?.import) throw new Error("the fixture has a credential item")
    const migration: ModuleMigration = {
      ...base,
      module: "gitlab",
      moduleName: "GitLab",
      plugin: "deckpilot.gitlab",
      pluginName: "GitLab",
      consent: [
        {
          ...token,
          import: { ...token.import, hosts: ["gitlab.com"], userOrigin: "apiUrl" },
        },
        {
          key: "network:user-origin:apiUrl",
          permission: {
            key: "network:user-origin:apiUrl",
            kind: "network",
            tier: "high",
            status: "notGranted",
            scope: { type: "userOriginSlot", methods: ["GET"], privateNetwork: true },
          },
          origin: {
            setting: "apiUrl",
            origin: server,
            methods: ["GET"],
            credentials: ["token"],
            opensPages: true,
          },
          why: "The built-in gitlab talked to this server.",
          decision: "pending",
        },
        {
          key: "network:user-origin:apiUrl:private-network",
          permission: {
            key: "network:user-origin:apiUrl:private-network",
            kind: "network",
            tier: "high",
            status: "notGranted",
            scope: { type: "privateNetwork" },
          },
          origin: { setting: "apiUrl", origin: server, privateNetwork: true },
          why: "The built-in gitlab was allowed to reach this server on a private network.",
          decision: "pending",
        },
      ],
    }
    const onDecide = vi.fn(() => Promise.resolve())
    const { user } = renderWithApp(
      <MigrationConsentDialog migration={migration} onDecide={onDecide} onClose={vi.fn()} />,
    )
    const dialog = within(
      await screen.findByRole("dialog", { name: "GitLab actions now run as a plugin" }),
    )
    const [credential, origin, network] = dialog.getAllByRole("listitem")
    if (!credential || !origin || !network) throw new Error("three items")
    // The credential: where it may go, by name.
    expect(
      within(credential).getByText("gitlab.com and the server you allow in apiUrl"),
    ).toBeInTheDocument()
    // The server: exactly it, what goes there, and its pages.
    expect(within(origin).getByText(`Connect to ${server}`)).toBeInTheDocument()
    const facts = within(within(origin).getByLabelText("The server"))
    expect(facts.getByText(server)).toBeInTheDocument()
    expect(facts.getByText("GET, to exactly this server and path")).toBeInTheDocument()
    expect(facts.getByText("token")).toBeInTheDocument()
    expect(facts.getByText("of https://gitlab.internal, in actions you start")).toBeInTheDocument()
    // The private network: an item of its own, with its own decision.
    expect(within(network).getByText(`Reach ${server} on your private network`)).toBeInTheDocument()
    expect(within(network).getByText("High risk")).toBeInTheDocument()
    expect(within(network).queryByText("Requests")).not.toBeInTheDocument()

    const confirm = dialog.getByRole("button", { name: "Confirm" })
    await user.click(within(credential).getByRole("radio", { name: "Allow" }))
    await user.click(within(origin).getByRole("radio", { name: "Allow" }))
    // Allowing the server does not allow its private network.
    expect(confirm).toBeDisabled()
    expect(dialog.getByText("1 of 3 items still need your decision.")).toBeInTheDocument()
    await user.click(within(network).getByRole("radio", { name: "Allow" }))
    await user.click(confirm)
    expect(onDecide).toHaveBeenCalledWith(
      [
        "credentials:token",
        "network:user-origin:apiUrl",
        "network:user-origin:apiUrl:private-network",
      ],
      [],
    )
  })

  it("decides nothing on Not now", async () => {
    const { user, client } = renderMarketplace({ migrations: [githubMigration()] })
    await user.click((await entry("GitHub")).getByRole("button", { name: /Review/ }))
    const dialog = within(await screen.findByRole("dialog"))
    await user.click(dialog.getByRole("radio", { name: "Allow" }))
    await user.click(dialog.getByRole("button", { name: "Not now" }))
    expect(client.decideModuleMigration).not.toHaveBeenCalled()
    expect((await entry("GitHub")).getByText("Waiting for your consent")).toBeInTheDocument()
  })

  it("renders only the named fields of an answer, never anything else it carries", () => {
    const migration = githubMigration()
    const [item] = migration.consent ?? []
    if (!item?.import) throw new Error("an import")
    // A broken or hostile answer: a value where none belongs.
    Object.assign(item.import, { secret: GITHUB_SECRET, value: GITHUB_SECRET })
    Object.assign(item, { secret: GITHUB_SECRET })
    render(<MigrationConsentDialog migration={migration} onDecide={vi.fn()} onClose={vi.fn()} />)
    expect(screen.getByRole("dialog")).not.toHaveTextContent(GITHUB_SECRET)
    expect(screen.getByRole("dialog")).toHaveTextContent("github.token")
  })

  it("shows the agent's refusal and stays open", async () => {
    const onDecide = vi.fn(() => Promise.reject(new Error("the plugin store refused")))
    const onClose = vi.fn()
    const { user } = renderWithApp(
      <MigrationConsentDialog
        migration={githubMigration()}
        onDecide={onDecide}
        onClose={onClose}
      />,
    )
    await user.click(screen.getByRole("radio", { name: "Allow" }))
    await user.click(screen.getByRole("button", { name: "Confirm" }))
    expect(await screen.findByRole("alert")).toHaveTextContent("the plugin store refused")
    expect(onClose).not.toHaveBeenCalled()
  })
})

describe("the bundled origin", () => {
  it("tells bundled plugins apart from the ones the user installed", () => {
    const details = (origin: PluginSummary["origin"]) =>
      ({ summary: { origin } }) as unknown as PluginDetails
    expect(isBundledPlugin(details({ type: "bundled" }))).toBe(true)
    expect(isBundledPlugin(details({ type: "sideload" }))).toBe(false)
    expect(isBundledPlugin(details({ type: "registry", registry: "r" }))).toBe(false)
  })
})

// ---- Legacy action types in the editors ----

const LEGACY = "github.pull-requests"
const TARGET = "deckpilot.github.pull-requests"

const builtinType: ActionType = {
  id: LEGACY,
  name: "Pull requests",
  description: "The built-in's pull requests.",
  category: "GitHub",
  parameters: [
    { key: "repo", label: "Repository", kind: { type: "text", multiline: false }, required: true },
  ],
  requiresProject: false,
}

const pluginType: ActionType = {
  ...builtinType,
  id: TARGET,
  description: "Lists open pull requests (plugin).",
}

function github(): ModuleInfo {
  return moduleInfo({ id: "github", name: "GitHub", actionTypes: [builtinType] })
}

function githubPlugin(state: "enabled" | "disabled" = "enabled"): PluginSummary {
  return {
    id: "deckpilot.github",
    name: "GitHub",
    publisher: "DeckPilot",
    version: "0.1.0",
    state,
    origin: { type: "bundled" },
    actions: [
      {
        action: "pull-requests",
        actionType: TARGET,
        name: "Pull requests",
        available: state === "enabled",
        ...(state === "enabled" ? {} : { unavailableReason: "the plugin is disabled" }),
        definition: pluginType,
      },
    ],
  }
}

function editorModules(
  migration: ModuleMigration,
  plugins: PluginSummary[] = [githubPlugin()],
): ModuleInfo[] {
  return withPluginModules([...builtInModules(), github()], pluginModules(plugins, [], [migration]))
}

describe("legacy action types a plugin replaces", () => {
  beforeEach(() => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
  })

  it("show under their module with the plugin's definition, deprecated, never twice in the picker", () => {
    const modules = editorModules(githubMigration({ phase: "cutOver" }))
    const builtin = modules.find((m) => m.id === "github")
    const legacy = builtin?.actionTypes.find((t) => t.id === LEGACY)
    expect(legacy).toEqual({ ...pluginType, id: LEGACY, deprecated: true })
    expect(builtin?.legacy?.[LEGACY]).toEqual({
      target: TARGET,
      plugin: "deckpilot.github",
      pluginName: "GitHub",
      unavailable: null,
    })
    // Only the plugin's own type is offered for new actions.
    const offered = modules.flatMap((m) => creatableTypes(m.actionTypes)).map((t) => t.id)
    expect(offered).toContain(TARGET)
    expect(offered).not.toContain(LEGACY)
    // Saved actions of the legacy type keep working.
    expect(typeAvailability(modules, LEGACY)).toMatchObject({ available: true, reason: null })
    expect(pluginActionUnavailable(modules, LEGACY)).toBeNull()
    // The workflow editor gets the same: deprecated legacy, the plugin's type once.
    const types = withPluginTypes([builtinType], modules)
    expect(types.map((t) => [t.id, Boolean(t.deprecated)])).toEqual([
      [LEGACY, true],
      [TARGET, false],
    ])
  })

  it("are unchanged while the module is routed to its built-in", () => {
    for (const phase of ["awaitingConsent", "failed", "notEligible"] as const) {
      const modules = editorModules(githubMigration({ phase }), [githubPlugin("disabled")])
      const builtin = modules.find((m) => m.id === "github")
      expect(builtin?.actionTypes).toEqual([builtinType])
      expect(builtin?.legacy).toBeUndefined()
      expect(typeAvailability(modules, LEGACY)).toMatchObject({ available: true, reason: null })
    }
  })

  it("are unavailable with the reason when the user withdrew the plugin", () => {
    const reason = "GitHub (deckpilot.github) was disabled by you"
    const modules = editorModules(
      githubMigration({ phase: "cutOver", route: "pluginWithdrawn", routeReason: reason }),
      [githubPlugin("disabled")],
    )
    const why = `GitHub (deckpilot.github): ${reason}`
    expect(typeAvailability(modules, LEGACY)).toMatchObject({ available: false, reason: why })
    expect(pluginActionUnavailable(modules, LEGACY)).toBe(why)
    // Also when the plugin was uninstalled and is known only by name.
    const gone = editorModules(
      githubMigration({ phase: "cutOver", route: "pluginWithdrawn", routeReason: reason }),
      [],
    )
    expect(pluginActionUnavailable(gone, LEGACY)).toBe(why)
  })

  it("mark an existing action as deprecated in the action editor", async () => {
    const pr = action("a-pr", "My pull requests", LEGACY, {
      parameters: { repo: { type: "text", value: "knittingcode/deckpilot" } },
    })
    api.getProfile.mockResolvedValue(profileView({ actions: [pr] }))
    const modules = [...builtInModules(), github()]
    const { user, titlebarAction } = renderWithApp(<ActionsPage />, {
      modules,
      pluginModules: pluginModules([githubPlugin()], [], [githubMigration({ phase: "cutOver" })]),
    })
    await user.click(await screen.findByRole("button", { name: /My pull requests/ }))
    expect(screen.getByTestId("legacy-alias")).toHaveTextContent(
      `Deprecated type: GitHub moved to the plugin GitHub, which runs it as ${TARGET}`,
    )
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled()
    expect(screen.getByDisplayValue("knittingcode/deckpilot")).toBeInTheDocument()

    // The picker offers the plugin's type, not the legacy one.
    titlebarAction("actions.new")
    const chooser = within(screen.getByRole("group", { name: "Action types" }))
    expect(chooser.queryByRole("region", { name: "GitHub" })).toBeTruthy()
    const buttons = chooser.getAllByRole("button", { name: /Pull requests/ })
    expect(buttons).toHaveLength(1)
  })
})

function Probe() {
  const modules = usePluginModules("agent")
  return (
    <p aria-label="plugins">
      {modules.map((m) => `${m.id}:${String(m.plugin?.aliases?.length ?? 0)}`).join(",")}
    </p>
  )
}

describe("asking the agent about migrations", () => {
  it("asks nothing without the flag", async () => {
    const client = createFakePluginClient({ migrations: [githubMigration()] })
    render(
      <PluginClientContext value={client}>
        <Probe />
      </PluginClientContext>,
    )
    await act(() => Promise.resolve())
    expect(client.listModuleMigrations).not.toHaveBeenCalled()
    expect(client.decideModuleMigration).not.toHaveBeenCalled()
    expect(client.retryModuleMigration).not.toHaveBeenCalled()
    expect(client.listPlugins).not.toHaveBeenCalled()
    expect(client.subscribe).not.toHaveBeenCalled()
    expect(screen.getByLabelText("plugins")).toHaveTextContent("")
  })

  it("reads the aliases with the flag and again after a migration event", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const client = createFakePluginClient({ migrations: [githubMigration({ phase: "cutOver" })] })
    render(
      <PluginClientContext value={client}>
        <Probe />
      </PluginClientContext>,
    )
    // The plugin is not installed in the fake: it is known by its migration.
    expect(await screen.findByText("deckpilot.github:1")).toBeInTheDocument()
    expect(client.listModuleMigrations).toHaveBeenCalledTimes(1)
    act(() => {
      client.emit({
        type: "migrationChanged",
        module: "github",
        plugin: "deckpilot.github",
        phase: "cutOver",
      })
    })
    await waitFor(() => {
      expect(client.listModuleMigrations).toHaveBeenCalledTimes(2)
    })
  })
})
