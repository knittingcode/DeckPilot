// Rendering inside the app's providers, for tests.

import { act, render } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { vi } from "vitest"

import { SidebarProvider } from "@/components/ui/sidebar"
import { TooltipProvider } from "@/components/ui/tooltip"
import { TitlebarContext } from "@/hooks/use-titlebar"
import { AppContext, type AppContextValue } from "@/lib/app-context"
import { TitlebarStore } from "@/lib/titlebar"

import { profiles } from "./fixtures"

/** An app context with spies for every callback; override what a test needs. */
export function appContext(overrides: Partial<AppContextValue> = {}): AppContextValue {
  return {
    profiles: profiles(),
    editingProfileId: "profile-1",
    setEditingProfileId: vi.fn(),
    refresh: vi.fn(),
    revision: 0,
    agent: null,
    setAgent: vi.fn(),
    updates: null,
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

export interface AppRender {
  context: AppContextValue
  titlebar: TitlebarStore
  user: ReturnType<typeof userEvent.setup>
  /** Runs a command the page put into the titlebar, e.g. `actions.new`. */
  titlebarAction: (id: string) => void
  rerender: (ui: ReactNode, context?: AppContextValue) => void
}

/**
 * Renders `ui` like a page of the main window: with the app context, a
 * titlebar store (pages contribute their commands to it), tooltips and the
 * sidebar.
 */
export function renderWithApp(ui: ReactNode, overrides: Partial<AppContextValue> = {}): AppRender {
  const context = appContext(overrides)
  const titlebar = new TitlebarStore()
  const wrap = (node: ReactNode, value: AppContextValue) => (
    <AppContext value={value}>
      <TitlebarContext value={titlebar}>
        <TooltipProvider>
          <SidebarProvider>{node}</SidebarProvider>
        </TooltipProvider>
      </TitlebarContext>
    </AppContext>
  )
  const result = render(wrap(ui, context))
  return {
    context,
    titlebar,
    user: userEvent.setup(),
    titlebarAction: (id) => {
      const action = titlebar.getSnapshot().actions?.find((a) => a.id === id)
      if (!action) throw new Error(`No titlebar action ${id}`)
      act(() => {
        action.onSelect?.()
      })
    },
    rerender: (node, value = context) => {
      result.rerender(wrap(node, value))
    },
  }
}
