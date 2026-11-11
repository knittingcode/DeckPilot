// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { AppStatusBar } from "@/components/status-bar"
import { CoreStatusItems } from "@/components/status-sources"
import { StatusContext, createStatusStores } from "@/hooks/use-status"
import { NOTIFICATION_EVENT } from "@/lib/api"
import { AppContext } from "@/lib/app-context"
import { SecurityContext } from "@/lib/security-context"
import type { ActionNotification, SecurityView } from "@/lib/types"
import { installDomStubs } from "@/test/dom"
import { agentConnection } from "@/test/fixtures"
import { appContext } from "@/test/render"
import { emitBackend, resetBackend } from "@/test/tauri"

vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

const security = {
  view: { problem: null, legacyCopies: [] } as unknown as SecurityView,
  setView: vi.fn(),
  refresh: vi.fn(),
}

beforeAll(installDomStubs)
beforeEach(resetBackend)
afterEach(cleanup)

async function renderStatusBar() {
  const stores = createStatusStores()
  render(
    <AppContext value={appContext({ agent: agentConnection() })}>
      <SecurityContext value={security}>
        <StatusContext value={stores}>
          <AppStatusBar stores={stores} />
          <CoreStatusItems />
        </StatusContext>
      </SecurityContext>
    </AppContext>,
  )
  // Listeners register asynchronously.
  await waitFor(() => {
    expect(emitBackend(NOTIFICATION_EVENT, { statusBar: false })).toBeGreaterThan(0)
  })
  return stores
}

function notify(notification: Partial<ActionNotification>) {
  act(() => {
    emitBackend(NOTIFICATION_EVENT, {
      title: "Notice",
      severity: "info",
      desktop: false,
      statusBar: true,
      ...notification,
    })
  })
}

describe("status bar notifications", () => {
  it("counts unread notifications, lists them newest first and marks them read", async () => {
    const user = userEvent.setup()
    await renderStatusBar()
    expect(screen.getByRole("button", { name: "Notifications" })).toBeInTheDocument()

    notify({ title: "Deploy finished", body: "3 nodes succeeded", severity: "success" })
    notify({ title: "Disk almost full", severity: "warning" })
    notify({ title: "Backup failed", severity: "error" })
    notify({ title: "Desktop only", severity: "error", statusBar: false })

    const bell = screen.getByRole("button", { name: "Notifications, 3 unread" })
    await user.click(bell)
    const list = within(screen.getByRole("list", { name: "Notifications" }))
    const items = list.getAllByRole("listitem")
    expect(items.map((li) => li.querySelector(".font-medium")?.textContent)).toEqual([
      "Backup failed",
      "Disk almost full",
      "Deploy finished",
    ])
    // Severity is conveyed in words, not only by color; success is shown as info.
    expect(within(items[0] as HTMLElement).getByLabelText("error")).toBeInTheDocument()
    expect(within(items[2] as HTMLElement).getByLabelText("info")).toBeInTheDocument()
    expect(list.getByText("3 nodes succeeded")).toBeInTheDocument()

    await user.keyboard("{Escape}")
    const read = screen.getByRole("button", { name: "Notifications" })
    expect(read).toHaveFocus()
  })

  it("clears the list", async () => {
    const user = userEvent.setup()
    const stores = await renderStatusBar()
    notify({ title: "Build failed", severity: "error" })
    await user.click(screen.getByRole("button", { name: "Notifications, 1 unread" }))
    await user.click(screen.getByRole("button", { name: "Clear" }))
    expect(screen.getByText("No notifications")).toBeInTheDocument()
    expect(stores.notifications.getSnapshot()).toEqual([])
  })
})
