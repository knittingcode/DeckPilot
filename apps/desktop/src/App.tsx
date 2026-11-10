import { listen } from "@tauri-apps/api/event"
import { Suspense, lazy, useCallback, useEffect, useMemo, useState } from "react"

import { AppSidebar } from "@/components/app-sidebar"
import { ErrorNotice, Loading } from "@/components/page"
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useAsync } from "@/hooks/use-async"
import { AGENT_STATUS_EVENT, UPDATE_STATE_EVENT, api } from "@/lib/api"
import { AppContext, type AppContextValue, type PageId } from "@/lib/app-context"
import { ThemeProvider } from "@/components/theme-provider"
import type { AgentConnection, UpdateState } from "@/lib/types"
// Pages load on first use, which keeps the start-up bundle small.
const PAGES: Record<PageId, React.LazyExoticComponent<() => React.JSX.Element>> = {
  dashboard: lazy(() => import("@/pages/dashboard").then((m) => ({ default: m.DashboardPage }))),
  actions: lazy(() => import("@/pages/actions").then((m) => ({ default: m.ActionsPage }))),
  workflows: lazy(() => import("@/pages/workflows").then((m) => ({ default: m.WorkflowsPage }))),
  profiles: lazy(() => import("@/pages/profiles").then((m) => ({ default: m.ProfilesPage }))),
  projects: lazy(() => import("@/pages/projects").then((m) => ({ default: m.ProjectsPage }))),
  models: lazy(() => import("@/pages/models").then((m) => ({ default: m.ModelsPage }))),
  designer: lazy(() =>
    import("@/pages/layout-designer").then((m) => ({ default: m.LayoutDesignerPage })),
  ),
  devices: lazy(() => import("@/pages/devices").then((m) => ({ default: m.DevicesPage }))),
  settings: lazy(() => import("@/pages/settings").then((m) => ({ default: m.SettingsPage }))),
}

const EDITING_KEY = "deckpilot.editingProfile"

function storedEditingProfile(): string | null {
  try {
    return localStorage.getItem(EDITING_KEY)
  } catch {
    return null
  }
}

function Studio() {
  const [page, setPage] = useState<PageId>("dashboard")
  const [revision, setRevision] = useState(0)
  const [agent, setAgent] = useState<AgentConnection | null>(null)
  const [updates, setUpdates] = useState<UpdateState | null>(null)
  const [editing, setEditing] = useState<string | null>(storedEditingProfile)
  const { state } = useAsync(() => api.listProfiles(), String(revision))

  const refresh = useCallback(() => {
    setRevision((value) => value + 1)
  }, [])

  const setEditingProfileId = useCallback((id: string) => {
    setEditing(id)
    try {
      localStorage.setItem(EDITING_KEY, id)
    } catch {
      // Remembered for this session only.
    }
  }, [])

  // Agent state changes (button presses, hotplug, tray) and update progress
  // arrive as backend events.
  useEffect(() => {
    api.agentStatus().then(setAgent, () => {
      setAgent({
        running: false,
        status: null,
        canStart: false,
        compatibility: null,
        appVersion: "",
      })
    })
    api.updateState().then(setUpdates, () => undefined)
    const unlistenAgent = listen<AgentConnection>(AGENT_STATUS_EVENT, (event) => {
      setAgent(event.payload)
      refresh()
    })
    const unlistenUpdates = listen<UpdateState>(UPDATE_STATE_EVENT, (event) => {
      setUpdates(event.payload)
    })
    return () => {
      void unlistenAgent.then((stop) => {
        stop()
      })
      void unlistenUpdates.then((stop) => {
        stop()
      })
    }
  }, [refresh])

  const context = useMemo<AppContextValue | null>(() => {
    if (state.status !== "ready") return null
    const profiles = state.data
    const valid = profiles.profiles.some((p) => p.id === editing)
    const editingProfileId = valid
      ? editing
      : (profiles.activeProfileId ?? profiles.profiles[0]?.id ?? null)
    return {
      profiles,
      editingProfileId,
      setEditingProfileId,
      refresh,
      revision,
      agent,
      setAgent,
      updates,
      setUpdates,
      navigate: setPage,
    }
  }, [state, editing, setEditingProfileId, refresh, revision, agent, updates])

  if (!context) {
    return (
      <div className="p-6">
        {state.status === "error" ? <ErrorNotice error={state.error} /> : <Loading />}
      </div>
    )
  }

  const Current = PAGES[page]
  return (
    <AppContext value={context}>
      <SidebarProvider>
        <AppSidebar current={page} onNavigate={setPage} />
        <SidebarInset>
          <Suspense
            fallback={
              <div className="p-6">
                <Loading />
              </div>
            }
          >
            <Current />
          </Suspense>
        </SidebarInset>
      </SidebarProvider>
    </AppContext>
  )
}

export function App() {
  return (
    <ThemeProvider>
      <TooltipProvider>
        <Studio />
        <Toaster position="bottom-right" />
      </TooltipProvider>
    </ThemeProvider>
  )
}
