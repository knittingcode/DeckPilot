// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { Server } from "lucide-react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { AppStatusBar } from "@/components/status-bar"
import { CoreStatusItems } from "@/components/status-sources"
import { StatusContext, type StatusStores, createStatusStores } from "@/hooks/use-status"
import { AppContext, type AppContextValue } from "@/lib/app-context"
import { SecurityContext } from "@/lib/security-context"
import {
  FINISHED_VISIBLE_MS,
  NOTIFICATION_LIMIT,
  NotificationStore,
  OperationStore,
  StatusItemStore,
  visibility,
} from "@/lib/status"
import { agentState, deviceSummary, updateItem } from "@/lib/status-model"
import { moduleInfo } from "@/lib/test-fixtures"
import type {
  AgentConnection,
  DeviceInfo,
  ModuleInfo,
  SecurityView,
  UpdateState,
} from "@/lib/types"
import { moduleStatusItems } from "@/modules"

/** Listeners of backend events, as the app registers them. */
const backend = new Map<string, (event: { payload: unknown }) => void>()
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    backend.set(name, handler)
    return Promise.resolve(() => {
      backend.delete(name)
    })
  },
}))

beforeAll(() => {
  if (!("ResizeObserver" in globalThis)) {
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    )
  }
})

afterEach(cleanup)

function device(serial: string, overrides: Partial<DeviceInfo> = {}): DeviceInfo {
  return {
    id: { serial } as DeviceInfo["id"],
    modelName: "Stream Deck MK.2",
    state: "connected",
    layout: { type: "none" } as unknown as DeviceInfo["layout"],
    page: null,
    source: "hid",
    ...overrides,
  }
}

function agent(overrides: Partial<NonNullable<AgentConnection["status"]>> = {}, running = true) {
  return {
    running,
    canStart: true,
    appVersion: "0.5.0",
    compatibility: { type: "compatible" },
    status: running
      ? {
          version: "0.5.0",
          protocol: 4,
          pid: 1,
          startedAt: 1_700_000_000,
          dataDir: "/data",
          logDir: "/logs",
          endpoint: "/sock",
          backend: "hid",
          profiles: [],
          activeProfile: { id: "p1", name: "Work" },
          activeProject: { id: "x", key: "SHOP", name: "Online Shop", resources: {} },
          devices: [device("A1")],
          brightness: 60,
          lastInputAt: null,
          ...overrides,
        }
      : null,
  } as AgentConnection
}

function updates(overrides: Partial<UpdateState> = {}): UpdateState {
  return {
    currentVersion: "0.5.0",
    configured: true,
    target: "darwin-aarch64",
    endpoint: "https://example.invalid",
    preferences: {} as UpdateState["preferences"],
    phase: "idle",
    available: null,
    progress: null,
    error: null,
    ...overrides,
  }
}

const security = {
  view: { problem: null, legacyCopies: [] } as unknown as SecurityView,
  setView: vi.fn(),
  refresh: vi.fn(),
}

function context(overrides: Partial<AppContextValue>): AppContextValue {
  return {
    profiles: { profiles: [], activeProfileId: null },
    editingProfileId: null,
    setEditingProfileId: vi.fn(),
    refresh: vi.fn(),
    revision: 0,
    agent: agent(),
    setAgent: vi.fn(),
    updates: updates(),
    setUpdates: vi.fn(),
    navigate: vi.fn(),
    openSettings: vi.fn(),
    closeSettings: vi.fn(),
    history: { canGoBack: false, canGoForward: false, back: vi.fn(), forward: vi.fn() },
    modules: [],
    setModules: vi.fn(),
    ...overrides,
  }
}

function Shell({
  stores,
  value,
  modules = [],
}: {
  stores: StatusStores
  value: AppContextValue
  modules?: ModuleInfo[]
}) {
  return (
    <AppContext value={value}>
      <SecurityContext value={security}>
        <StatusContext value={stores}>
          <AppStatusBar stores={stores} />
          <CoreStatusItems />
          {moduleStatusItems(modules).map(({ key, component: Item }) => (
            <Item key={key} />
          ))}
        </StatusContext>
      </SecurityContext>
    </AppContext>
  )
}

describe("status stores", () => {
  it("registers, orders and removes items", () => {
    const store = new StatusItemStore()
    const listener = vi.fn()
    store.subscribe(listener)
    store.set({ id: "a", owner: "core", placement: "left", priority: 50, label: "A" })
    store.set({ id: "b", owner: "core", placement: "left", priority: 90, label: "B" })
    store.set({ id: "c", owner: "core", placement: "right", priority: 90, label: "C" })
    expect(StatusItemStore.arrange(store.getSnapshot(), "left").map((i) => i.id)).toEqual([
      "b",
      "a",
    ])
    // Same appearance: no re-render.
    store.set({
      id: "a",
      owner: "core",
      placement: "left",
      priority: 50,
      label: "A",
      onSelect: vi.fn(),
    })
    expect(listener).toHaveBeenCalledTimes(3)
    store.remove("a")
    expect(store.getSnapshot().map((i) => i.id)).toEqual(["b", "c"])
    expect([visibility(100), visibility(60), visibility(10)]).toEqual(["always", "medium", "wide"])
  })

  it("tracks concurrent operations and drops finished ones after a while", () => {
    let now = 1000
    const store = new OperationStore(() => now)
    store.start({ id: "w1", kind: "workflow", title: "One" })
    now = 2000
    store.start({ id: "u", kind: "update", title: "Download" })
    store.update("u", { progress: 40 })
    now = 3000
    store.finish("w1", { error: "A step failed" })
    const ordered = OperationStore.ordered(store.getSnapshot())
    expect(ordered.map((o) => [o.id, o.status])).toEqual([
      ["u", "running"],
      ["w1", "failed"],
    ])
    expect(store.get("u")?.progress).toBe(40)
    // Progress of a finished operation does not change.
    store.update("w1", { progress: 10 })
    expect(store.get("w1")?.progress).toBeUndefined()
    now = 3000 + FINISHED_VISIBLE_MS + 1
    store.prune()
    expect(store.getSnapshot().map((o) => o.id)).toEqual(["u"])
  })

  it("combines repeated notifications and keeps a bounded list", () => {
    const store = new NotificationStore(() => 5)
    store.notify({ key: "agent.stopped", severity: "warning", title: "Stopped" })
    store.notify({ key: "agent.stopped", severity: "warning", title: "Stopped" })
    expect(store.getSnapshot()).toHaveLength(1)
    expect(store.get("agent.stopped")?.count).toBe(2)
    store.markAllRead()
    store.notify({ key: "agent.stopped", severity: "warning", title: "Stopped" })
    expect(store.get("agent.stopped")).toMatchObject({ count: 1, read: false })
    for (let i = 0; i < NOTIFICATION_LIMIT + 5; i++) {
      store.notify({ key: `n${i}`, severity: "info", title: `n${i}` })
    }
    expect(store.getSnapshot()).toHaveLength(NOTIFICATION_LIMIT)
    expect(NotificationStore.newestFirst(store.getSnapshot())[0]?.key).toBe(
      `n${NOTIFICATION_LIMIT + 4}`,
    )
  })
})

describe("status model", () => {
  it("reports the agent's real state", () => {
    expect(agentState(null)).toBe("connecting")
    expect(agentState(agent({}, false))).toBe("stopped")
    expect(agentState(agent())).toBe("running")
    expect(
      agentState({
        ...agent(),
        compatibility: { type: "agentOutdated", agent: "0.4.0", app: "0.5.0" },
      }),
    ).toBe("incompatible")
  })

  it("never counts simulated devices as physical ones", () => {
    expect(deviceSummary([])).toBe("No devices")
    expect(deviceSummary([device("A"), device("B", { state: "disconnected" })])).toBe("1 device")
    expect(deviceSummary([device("A"), device("S", { source: "mock" })])).toBe(
      "1 device · 1 simulated",
    )
    expect(deviceSummary([device("S", { source: "mock" })])).toBe("1 simulated")
  })

  it("follows the updater's phases", () => {
    expect(updateItem(updates({ configured: false }))).toBeNull()
    expect(updateItem(updates())).toBeNull()
    const available = { version: "0.6.0", notes: null, date: null }
    expect(updateItem(updates({ available }))?.label).toBe("Update 0.6.0")
    expect(
      updateItem(
        updates({ available, phase: "downloading", progress: { downloaded: 25, total: 100 } }),
      ),
    ).toEqual({ label: "Downloading 0.6.0", progress: 25 })
    expect(updateItem(updates({ available, phase: "ready" }))?.label).toBe(
      "Restart to update to 0.6.0",
    )
    expect(updateItem(updates({ error: "signature invalid" }))?.tone).toBe("danger")
  })
})

describe("status bar", () => {
  it("shows agent, devices, profile and the active project of an enabled module", () => {
    const stores = createStatusStores()
    const projects = moduleInfo({ id: "projects" })
    const { rerender } = render(<Shell stores={stores} value={context({})} modules={[projects]} />)
    const bar = within(screen.getByRole("contentinfo", { name: "Status bar" }))
    expect(bar.getByText("Agent")).toBeInTheDocument()
    expect(bar.getByText("1 device")).toBeInTheDocument()
    expect(bar.getByText("Work")).toBeInTheDocument()
    expect(bar.getByText("SHOP")).toBeInTheDocument()

    // Disabling the module removes its item.
    rerender(
      <Shell stores={stores} value={context({})} modules={[{ ...projects, enabled: false }]} />,
    )
    expect(bar.queryByText("SHOP")).not.toBeInTheDocument()

    // Switching the profile on the device: the project shown is the new one's.
    const switched = agent({
      activeProfile: { id: "p2", name: "Home" },
      activeProject: { id: "y", key: "LAB", name: "Lab", resources: {} },
    })
    rerender(<Shell stores={stores} value={context({ agent: switched })} modules={[projects]} />)
    expect(bar.getByText("Home")).toBeInTheDocument()
    expect(bar.getByText("LAB")).toBeInTheDocument()
    expect(bar.queryByText("SHOP")).not.toBeInTheDocument()

    // A profile without an active project shows none.
    rerender(
      <Shell
        stores={stores}
        value={context({ agent: agent({ activeProject: null }) })}
        modules={[projects]}
      />,
    )
    expect(bar.queryByText("LAB")).not.toBeInTheDocument()
  })

  it("lists the notifications workflow actions show", async () => {
    const stores = createStatusStores()
    render(<Shell stores={stores} value={context({})} />)
    await act(async () => {
      await Promise.resolve()
    })
    const emit = backend.get("agent-notification")
    expect(emit).toBeDefined()
    act(() => {
      emit?.({
        payload: {
          title: "<b>Start work</b>",
          body: "3 succeeded",
          severity: "success",
          desktop: true,
          statusBar: true,
        },
      })
      emit?.({
        payload: { title: "Hidden", severity: "error", desktop: true, statusBar: false },
      })
    })
    const shown = stores.notifications.getSnapshot()
    expect(shown.map((n) => n.title)).toEqual(["<b>Start work</b>"])
    expect(shown[0]?.severity).toBe("info")
    expect(shown[0]?.message).toBe("3 succeeded")
  })

  it("notifies once when the agent stops and when a device disconnects", () => {
    const stores = createStatusStores()
    const { rerender } = render(<Shell stores={stores} value={context({})} />)
    rerender(<Shell stores={stores} value={context({ agent: agent({ devices: [] }) })} />)
    expect(stores.notifications.get("device.A1.disconnected")?.title).toBe(
      "Stream Deck MK.2 disconnected",
    )
    rerender(<Shell stores={stores} value={context({ agent: agent({}, false) })} />)
    rerender(<Shell stores={stores} value={context({ agent: agent({}, false) })} />)
    expect(stores.notifications.get("agent.stopped")?.count).toBe(1)
    expect(screen.getByText("Agent stopped")).toBeInTheDocument()
    expect(screen.queryByText("1 device")).not.toBeInTheDocument()
    // Reconnecting shows the agent again.
    rerender(<Shell stores={stores} value={context({ agent: agent() })} />)
    expect(screen.getByText("Agent")).toBeInTheDocument()
  })

  it("shows update download progress as an operation and failures as notifications", () => {
    const stores = createStatusStores()
    const available = { version: "0.6.0", notes: null, date: null }
    const { rerender } = render(
      <Shell stores={stores} value={context({ updates: updates({ available }) })} />,
    )
    expect(screen.getByRole("button", { name: /Update 0.6.0/ })).toBeInTheDocument()
    expect(stores.notifications.get("update.available.0.6.0")).toBeDefined()

    rerender(
      <Shell
        stores={stores}
        value={context({
          updates: updates({
            available,
            phase: "downloading",
            progress: { downloaded: 30, total: 100 },
          }),
        })}
      />,
    )
    expect(stores.operations.get("update.download")).toMatchObject({
      status: "running",
      progress: 30,
    })
    rerender(
      <Shell
        stores={stores}
        value={context({ updates: updates({ available, error: "signature invalid" }) })}
      />,
    )
    expect(stores.operations.get("update.download")?.status).toBe("failed")
    expect(stores.notifications.get("update.failed")?.message).toBe("signature invalid")
  })

  it("lists concurrent operations and notifications in keyboard-accessible popovers", async () => {
    const stores = createStatusStores()
    render(<Shell stores={stores} value={context({})} />)
    act(() => {
      stores.operations.start({ id: "a", kind: "workflow", title: "Running “Start Work”" })
      stores.operations.start({ id: "b", kind: "update", title: "Downloading DeckPilot 0.6.0" })
      stores.notifications.notify({ key: "x", severity: "error", title: "Workflow failed" })
    })
    const activity = screen.getByRole("button", { name: /Background activity: .*1 more/ })
    activity.focus()
    await userEvent.keyboard("{Enter}")
    expect(await screen.findByText("Background activity")).toBeInTheDocument()
    expect(screen.getAllByText("Downloading DeckPilot 0.6.0").length).toBeGreaterThan(0)
    await userEvent.keyboard("{Escape}")
    expect(screen.queryByText("Background activity")).not.toBeInTheDocument()

    await userEvent.click(screen.getByRole("button", { name: "Notifications, 1 unread" }))
    expect(
      within(screen.getByRole("list", { name: "Notifications" })).getByText("Workflow failed"),
    ).toBeInTheDocument()
    await userEvent.keyboard("{Escape}")
    expect(screen.getByRole("button", { name: "Notifications" })).toBeInTheDocument()
  })

  it("moves lower-priority items into the overflow menu", async () => {
    const stores = createStatusStores()
    render(<Shell stores={stores} value={context({})} />)
    act(() => {
      stores.items.set({
        id: "test.low",
        owner: "test",
        placement: "right",
        priority: 10,
        icon: Server,
        label: "Low priority",
        onSelect: vi.fn(),
      })
    })
    const inline = screen.getByRole("button", { name: "Low priority" })
    expect(inline).toHaveClass("hidden")
    await userEvent.click(screen.getByRole("button", { name: "More status" }))
    expect(await screen.findByRole("menuitem", { name: "Low priority" })).toBeInTheDocument()
  })
})
